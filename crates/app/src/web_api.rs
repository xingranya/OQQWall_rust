use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use rand::RngCore;
use oqqwall_rust_core::{
    Command, GlobalAction, GlobalActionCommand, Id128, ReviewAction, ReviewActionCommand,
    StateView,
};
use oqqwall_rust_core::state::PostStage;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::engine::EngineHandle;

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

const FULL_PERMISSIONS: [&str; 7] = [
    "review.read",
    "review.write",
    "send.execute",
    "blacklist.read",
    "blacklist.write",
    "session.manage",
    "token.manage",
];
const DEFAULT_SESSION_TTL_SEC: i64 = 12 * 60 * 60;

#[derive(Clone)]
struct ApiState {
    cmd_tx: tokio::sync::mpsc::Sender<Command>,
    state: Arc<RwLock<StateView>>,
    auth: Arc<RwLock<AuthStore>>,
    tz_offset_minutes: i32,
}

#[derive(Debug, Clone)]
struct ApiToken {
    token_id: String,
    permissions: BTreeSet<String>,
    expire_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct ApiSession {
    session_id: String,
    token_id: String,
    permissions: BTreeSet<String>,
    expires_at: i64,
}

#[derive(Debug, Default)]
struct AuthStore {
    tokens: HashMap<String, ApiToken>,
    sessions: HashMap<String, ApiSession>,
    idempotency_seen: HashSet<String>,
    next_token_seq: u64,
}

impl AuthStore {
    fn new(root_token: String) -> Self {
        let mut tokens = HashMap::new();
        let permissions = FULL_PERMISSIONS.iter().map(|v| (*v).to_string()).collect();
        tokens.insert(
            root_token,
            ApiToken {
                token_id: "root".to_string(),
                permissions,
                expire_at: None,
            },
        );
        Self {
            tokens,
            sessions: HashMap::new(),
            idempotency_seen: HashSet::new(),
            next_token_seq: 1,
        }
    }
}

#[derive(Debug, Clone)]
struct AuthContext {
    session_id: String,
    token_id: String,
}

#[derive(Serialize)]
struct ApiError {
    error: ApiErrorBody,
}

#[derive(Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
    request_id: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    token: String,
}

#[derive(Serialize)]
struct LoginResponse {
    session_id: String,
    expires_at: i64,
    permissions: Vec<String>,
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    permissions: Vec<String>,
    #[serde(default)]
    expire_at: Option<i64>,
}

#[derive(Serialize)]
struct CreateTokenResponse {
    token: String,
    token_id: String,
    expire_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ListPostsQuery {
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    cursor: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct PostListItem {
    post_id: String,
    review_id: Option<String>,
    group_id: String,
    stage: String,
    external_code: Option<u64>,
    internal_code: Option<u32>,
    sender_id: Option<String>,
    created_at_ms: i64,
    last_error: Option<String>,
}

#[derive(Serialize)]
struct ListPostsResponse {
    items: Vec<PostListItem>,
    next_cursor: Option<usize>,
}

#[derive(Serialize)]
struct PostDetailResponse {
    post_id: String,
    review_id: Option<String>,
    review_code: Option<u32>,
    group_id: String,
    stage: String,
    external_code: Option<u64>,
    sender_id: Option<String>,
    session_id: String,
    created_at_ms: i64,
    is_anonymous: bool,
    is_safe: bool,
    blocks: Vec<PostBlock>,
    render_png_blob_id: Option<String>,
    last_error: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind")]
enum PostBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "attachment")]
    Attachment {
        media_kind: String,
        reference_type: String,
        reference: String,
        size_bytes: Option<u64>,
    },
}

#[derive(Deserialize)]
struct ReviewDecisionRequest {
    action: String,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    delay_ms: Option<i64>,
}

#[derive(Serialize)]
struct ReviewDecisionResponse {
    review_id: String,
    status: &'static str,
}

#[derive(Deserialize)]
struct ListBlacklistQuery {
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    cursor: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Serialize)]
struct BlacklistItem {
    group_id: String,
    sender_id: String,
    reason: Option<String>,
}

#[derive(Serialize)]
struct ListBlacklistResponse {
    items: Vec<BlacklistItem>,
    next_cursor: Option<usize>,
}

#[derive(Deserialize)]
struct CreateBlacklistRequest {
    group_id: String,
    sender_id: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct SendPostsRequest {
    post_ids: Vec<String>,
    mode: String,
    #[serde(default)]
    schedule_at: Option<i64>,
}

#[derive(Serialize)]
struct SendPostsResponse {
    accepted: usize,
    failed: Vec<SendFailure>,
}

#[derive(Serialize)]
struct SendFailure {
    post_id: String,
    reason: String,
}

pub fn spawn_web_api(handle: &EngineHandle, config: &AppConfig) {
    if !config.web_api_enabled {
        debug_log!("web api disabled by config");
        return;
    }
    let Some(root_token) = config.web_api_root_token.clone() else {
        debug_log!("web api disabled: missing api token");
        return;
    };
    if root_token.len() < 32 {
        debug_log!("web api disabled: api token too short");
        return;
    }

    let state = ApiState {
        cmd_tx: handle.cmd_tx.clone(),
        state: handle.state(),
        auth: Arc::new(RwLock::new(AuthStore::new(root_token))),
        tz_offset_minutes: config.tz_offset_minutes,
    };

    let app = Router::new()
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/sessions/:session_id/revoke", post(revoke_session))
        .route("/v1/auth/tokens", post(create_token))
        .route("/v1/posts", get(list_posts))
        .route("/v1/posts/:post_id", get(get_post))
        .route("/v1/reviews/:review_id/decision", post(decide_review))
        .route("/v1/blacklist", get(list_blacklist).post(create_blacklist))
        .route(
            "/v1/blacklist/:group_id/:sender_id",
            delete(delete_blacklist),
        )
        .route("/v1/posts/send", post(send_posts))
        .with_state(state);

    let bind_addr = format!("0.0.0.0:{}", config.web_api_port);
    tokio::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
            Ok(listener) => listener,
            Err(_err) => {
                debug_log!("web api bind failed {}: {}", bind_addr, _err);
                return;
            }
        };
        debug_log!("web api started: {}", bind_addr);
        if let Err(_err) = axum::serve(listener, app).await {
            debug_log!("web api stopped: {}", _err);
        }
    });
}

async fn login(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let now = now_sec();

    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    let Some(token) = guard.tokens.get(&req.token).cloned() else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid token",
            request_id,
        );
    };
    if token.expire_at.map(|ts| ts <= now).unwrap_or(false) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "token expired",
            request_id,
        );
    }

    let session_id = random_hex32();
    let expires_at = now + DEFAULT_SESSION_TTL_SEC;
    let session = ApiSession {
        session_id: session_id.clone(),
        token_id: token.token_id,
        permissions: token.permissions.clone(),
        expires_at,
    };
    guard.sessions.insert(session_id.clone(), session);

    let resp = LoginResponse {
        session_id,
        expires_at,
        permissions: token.permissions.into_iter().collect(),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

async fn logout(State(state): State<ApiState>, headers: HeaderMap) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, None, &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    guard.sessions.remove(&auth.session_id);
    StatusCode::NO_CONTENT.into_response()
}

async fn revoke_session(
    State(state): State<ApiState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("session.manage"), &request_id) {
        return resp;
    }
    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    guard.sessions.remove(&session_id);
    StatusCode::NO_CONTENT.into_response()
}

async fn create_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateTokenRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("token.manage"), &request_id) {
        return resp;
    }

    let mut permissions = BTreeSet::new();
    for permission in req.permissions {
        if !FULL_PERMISSIONS.contains(&permission.as_str()) {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                &format!("unsupported permission: {}", permission),
                request_id,
            );
        }
        permissions.insert(permission);
    }
    if permissions.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "permissions cannot be empty",
            request_id,
        );
    }

    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id,
            );
        }
    };
    guard.next_token_seq = guard.next_token_seq.saturating_add(1);
    let token_id = format!("tok_{}", guard.next_token_seq);
    let token = random_hex32();
    guard.tokens.insert(
        token.clone(),
        ApiToken {
            token_id: token_id.clone(),
            permissions,
            expire_at: req.expire_at,
        },
    );

    (
        StatusCode::OK,
        Json(CreateTokenResponse {
            token,
            token_id,
            expire_at: req.expire_at,
        }),
    )
        .into_response()
}

async fn list_posts(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListPostsQuery>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("review.read"), &request_id) {
        return resp;
    }

    let stage_filter = match query.stage.as_deref() {
        Some(value) => match parse_stage(value) {
            Some(stage) => Some(stage),
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "BAD_REQUEST",
                    "invalid stage",
                    request_id,
                );
            }
        },
        None => None,
    };

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let mut items = guard
        .posts
        .values()
        .filter(|meta| stage_filter.map(|stage| meta.stage == stage).unwrap_or(true))
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then_with(|| b.post_id.cmp(&a.post_id)));

    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let page = items.iter().skip(cursor).take(limit);
    let mut out = Vec::new();
    for meta in page {
        let review_code = meta
            .review_id
            .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));
        let sender_id = guard
            .session_ingress
            .get(&meta.session_id)
            .and_then(|ids| ids.first())
            .and_then(|id| guard.ingress_meta.get(id))
            .map(|ingress| ingress.user_id.clone());

        out.push(PostListItem {
            post_id: id_to_string(meta.post_id),
            review_id: meta.review_id.map(id_to_string),
            group_id: meta.group_id.clone(),
            stage: stage_to_string(meta.stage),
            external_code: guard.external_code_by_post.get(&meta.post_id).copied(),
            internal_code: review_code,
            sender_id,
            created_at_ms: meta.created_at_ms,
            last_error: meta.last_error.clone(),
        });
    }

    let next_cursor = if cursor + out.len() < items.len() {
        Some(cursor + out.len())
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(ListPostsResponse {
            items: out,
            next_cursor,
        }),
    )
        .into_response()
}

async fn get_post(
    State(state): State<ApiState>,
    Path(post_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("review.read"), &request_id) {
        return resp;
    }

    let Some(post_id) = parse_id128(&post_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "invalid post_id",
            request_id,
        );
    };

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let Some(meta) = guard.posts.get(&post_id) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "post not found",
            request_id,
        );
    };

    let review_code = meta
        .review_id
        .and_then(|id| guard.reviews.get(&id).map(|review| review.review_code));
    let sender_id = guard
        .session_ingress
        .get(&meta.session_id)
        .and_then(|ids| ids.first())
        .and_then(|id| guard.ingress_meta.get(id))
        .map(|ingress| ingress.user_id.clone());

    let blocks = guard
        .drafts
        .get(&post_id)
        .map(|draft| {
            draft
                .blocks
                .iter()
                .map(|block| match block {
                    oqqwall_rust_core::draft::DraftBlock::Paragraph { text } => {
                        PostBlock::Text { text: text.clone() }
                    }
                    oqqwall_rust_core::draft::DraftBlock::Attachment {
                        kind,
                        reference,
                        size_bytes,
                    } => {
                        let (reference_type, reference) = match reference {
                            oqqwall_rust_core::draft::MediaReference::RemoteUrl { url } => {
                                ("remote_url".to_string(), url.clone())
                            }
                            oqqwall_rust_core::draft::MediaReference::Blob { blob_id } => {
                                ("blob_id".to_string(), id_to_string(*blob_id))
                            }
                        };
                        PostBlock::Attachment {
                            media_kind: media_kind_to_string(*kind),
                            reference_type,
                            reference,
                            size_bytes: *size_bytes,
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let render_png_blob_id = guard
        .render
        .get(&post_id)
        .and_then(|render| render.png_blob)
        .map(id_to_string);

    (
        StatusCode::OK,
        Json(PostDetailResponse {
            post_id: id_to_string(meta.post_id),
            review_id: meta.review_id.map(id_to_string),
            review_code,
            group_id: meta.group_id.clone(),
            stage: stage_to_string(meta.stage),
            external_code: guard.external_code_by_post.get(&meta.post_id).copied(),
            sender_id,
            session_id: id_to_string(meta.session_id),
            created_at_ms: meta.created_at_ms,
            is_anonymous: meta.is_anonymous,
            is_safe: meta.is_safe,
            blocks,
            render_png_blob_id,
            last_error: meta.last_error.clone(),
        }),
    )
        .into_response()
}

async fn decide_review(
    State(state): State<ApiState>,
    Path(review_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ReviewDecisionRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("review.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let Some(review_id) = parse_id128(&review_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "invalid review_id",
            request_id,
        );
    };

    if let Some(key) = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        let dedup_key = format!("review:{}:{}:{}", auth.session_id, review_id.0, key);
        let mut auth_guard = match state.auth.write() {
            Ok(guard) => guard,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL",
                    "auth store unavailable",
                    request_id,
                );
            }
        };
        if auth_guard.idempotency_seen.contains(&dedup_key) {
            return (
                StatusCode::OK,
                Json(ReviewDecisionResponse {
                    review_id: id_to_string(review_id),
                    status: "applied",
                }),
            )
                .into_response();
        }
        auth_guard.idempotency_seen.insert(dedup_key);
    }

    let action = match req.action.as_str() {
        "approve" => ReviewAction::Approve,
        "reject" => ReviewAction::Reject,
        "defer" => ReviewAction::Defer {
            delay_ms: req.delay_ms.unwrap_or(0),
        },
        "skip" => ReviewAction::Skip,
        "blacklist" => ReviewAction::Blacklist {
            reason: req.comment.clone(),
        },
        "immediate" => ReviewAction::Immediate,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "unsupported action",
                request_id,
            );
        }
    };

    let cmd = Command::ReviewAction(ReviewActionCommand {
        review_id: Some(review_id),
        review_code: None,
        audit_msg_id: None,
        action,
        operator_id: format!("api:{}", auth.token_id),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });

    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
            request_id,
        );
    }

    (
        StatusCode::OK,
        Json(ReviewDecisionResponse {
            review_id: id_to_string(review_id),
            status: "applied",
        }),
    )
        .into_response()
}

async fn list_blacklist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListBlacklistQuery>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    if let Err(resp) = authenticate(&state, &headers, Some("blacklist.read"), &request_id) {
        return resp;
    }

    let guard = match state.state.read() {
        Ok(guard) => guard,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    let mut rows = Vec::new();
    for (group_id, group) in &guard.blacklist {
        if query
            .group_id
            .as_ref()
            .map(|selected| selected != group_id)
            .unwrap_or(false)
        {
            continue;
        }
        for (sender_id, reason) in group {
            rows.push(BlacklistItem {
                group_id: group_id.clone(),
                sender_id: sender_id.clone(),
                reason: reason.clone(),
            });
        }
    }
    rows.sort_by(|a, b| a.group_id.cmp(&b.group_id).then_with(|| a.sender_id.cmp(&b.sender_id)));

    let cursor = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = rows
        .iter()
        .skip(cursor)
        .take(limit)
        .map(|row| BlacklistItem {
            group_id: row.group_id.clone(),
            sender_id: row.sender_id.clone(),
            reason: row.reason.clone(),
        })
        .collect::<Vec<_>>();
    let next_cursor = if cursor + items.len() < rows.len() {
        Some(cursor + items.len())
    } else {
        None
    };

    (
        StatusCode::OK,
        Json(ListBlacklistResponse { items, next_cursor }),
    )
        .into_response()
}

async fn create_blacklist(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<CreateBlacklistRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("blacklist.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };
    let sender_id = req.sender_id.trim();
    if sender_id.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "BAD_REQUEST",
            "sender_id cannot be empty",
            request_id,
        );
    }
    let cmd = Command::GlobalAction(GlobalActionCommand {
        group_id: req.group_id,
        action: GlobalAction::BlacklistAdd {
            sender_id: sender_id.to_string(),
            reason: req
                .reason
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        },
        operator_id: format!("api:{}", auth.token_id),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
            request_id,
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn delete_blacklist(
    State(state): State<ApiState>,
    Path((group_id, sender_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("blacklist.write"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    let cmd = Command::GlobalAction(GlobalActionCommand {
        group_id,
        action: GlobalAction::BlacklistRemove {
            sender_id: sender_id.clone(),
        },
        operator_id: format!("api:{}", auth.token_id),
        now_ms: now_ms(),
        tz_offset_minutes: state.tz_offset_minutes,
    });
    if state.cmd_tx.send(cmd).await.is_err() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "UNAVAILABLE",
            "engine command channel closed",
            request_id,
        );
    }

    StatusCode::NO_CONTENT.into_response()
}

async fn send_posts(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<SendPostsRequest>,
) -> impl IntoResponse {
    let request_id = request_id(&headers);
    let auth = match authenticate(&state, &headers, Some("send.execute"), &request_id) {
        Ok(auth) => auth,
        Err(resp) => return resp,
    };

    if req.mode == "scheduled" && req.schedule_at.is_some() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNSUPPORTED",
            "schedule_at is not supported by current command model",
            request_id,
        );
    }

    let mut failed = Vec::new();
    let mut accepted = 0usize;
    let snapshot = match state.state.read() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "state unavailable",
                request_id,
            );
        }
    };

    for raw_post_id in req.post_ids {
        let Some(post_id) = parse_id128(&raw_post_id) else {
            failed.push(SendFailure {
                post_id: raw_post_id,
                reason: "invalid post_id".to_string(),
            });
            continue;
        };
        let Some(post) = snapshot.posts.get(&post_id) else {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "post not found".to_string(),
            });
            continue;
        };
        let Some(review_id) = post.review_id else {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "post has no review_id".to_string(),
            });
            continue;
        };

        let action = match req.mode.as_str() {
            "immediate" => ReviewAction::Immediate,
            "scheduled" => ReviewAction::Approve,
            _ => {
                failed.push(SendFailure {
                    post_id: id_to_string(post_id),
                    reason: "unsupported mode".to_string(),
                });
                continue;
            }
        };

        let cmd = Command::ReviewAction(ReviewActionCommand {
            review_id: Some(review_id),
            review_code: None,
            audit_msg_id: None,
            action,
            operator_id: format!("api:{}", auth.token_id),
            now_ms: now_ms(),
            tz_offset_minutes: state.tz_offset_minutes,
        });
        if state.cmd_tx.send(cmd).await.is_err() {
            failed.push(SendFailure {
                post_id: id_to_string(post_id),
                reason: "engine command channel closed".to_string(),
            });
            continue;
        }
        accepted = accepted.saturating_add(1);
    }

    (
        StatusCode::OK,
        Json(SendPostsResponse { accepted, failed }),
    )
        .into_response()
}

fn authenticate(
    state: &ApiState,
    headers: &HeaderMap,
    required_permission: Option<&str>,
    request_id: &str,
) -> Result<AuthContext, axum::response::Response> {
    let Some(token) = bearer_token(headers) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "missing bearer session",
            request_id.to_string(),
        ));
    };

    let mut guard = match state.auth.write() {
        Ok(guard) => guard,
        Err(_) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL",
                "auth store unavailable",
                request_id.to_string(),
            ));
        }
    };

    let now = now_sec();
    guard
        .sessions
        .retain(|_, session| session.expires_at > now);

    let Some(session) = guard.sessions.get(&token).cloned() else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "invalid or expired session",
            request_id.to_string(),
        ));
    };

    if let Some(permission) = required_permission {
        if !session.permissions.contains(permission) {
            return Err(error_response(
                StatusCode::FORBIDDEN,
                "PERMISSION_DENIED",
                "permission denied",
                request_id.to_string(),
            ));
        }
    }

    Ok(AuthContext {
        session_id: session.session_id,
        token_id: session.token_id,
    })
}

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &str,
    request_id: String,
) -> axum::response::Response {
    let body = ApiError {
        error: ApiErrorBody {
            code,
            message: message.to_string(),
            request_id,
        },
    };
    (status, Json(body)).into_response()
}

fn request_id(headers: &HeaderMap) -> String {
    headers
        .get("X-Request-Id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(random_hex32)
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("Authorization")?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

fn parse_id128(value: &str) -> Option<Id128> {
    value.parse::<u128>().ok().map(Id128)
}

fn parse_stage(value: &str) -> Option<PostStage> {
    match value {
        "drafted" => Some(PostStage::Drafted),
        "render_requested" => Some(PostStage::RenderRequested),
        "rendered" => Some(PostStage::Rendered),
        "review_pending" => Some(PostStage::ReviewPending),
        "reviewed" => Some(PostStage::Reviewed),
        "scheduled" => Some(PostStage::Scheduled),
        "sending" => Some(PostStage::Sending),
        "sent" => Some(PostStage::Sent),
        "rejected" => Some(PostStage::Rejected),
        "skipped" => Some(PostStage::Skipped),
        "manual" => Some(PostStage::Manual),
        "failed" => Some(PostStage::Failed),
        _ => None,
    }
}

fn stage_to_string(stage: PostStage) -> String {
    match stage {
        PostStage::Drafted => "drafted",
        PostStage::RenderRequested => "render_requested",
        PostStage::Rendered => "rendered",
        PostStage::ReviewPending => "review_pending",
        PostStage::Reviewed => "reviewed",
        PostStage::Scheduled => "scheduled",
        PostStage::Sending => "sending",
        PostStage::Sent => "sent",
        PostStage::Rejected => "rejected",
        PostStage::Skipped => "skipped",
        PostStage::Manual => "manual",
        PostStage::Failed => "failed",
    }
    .to_string()
}

fn media_kind_to_string(kind: oqqwall_rust_core::draft::MediaKind) -> String {
    match kind {
        oqqwall_rust_core::draft::MediaKind::Image => "image",
        oqqwall_rust_core::draft::MediaKind::Video => "video",
        oqqwall_rust_core::draft::MediaKind::File => "file",
        oqqwall_rust_core::draft::MediaKind::Audio => "audio",
        oqqwall_rust_core::draft::MediaKind::Other => "other",
        oqqwall_rust_core::draft::MediaKind::Sticker => "sticker",
    }
    .to_string()
}

fn id_to_string(id: Id128) -> String {
    id.0.to_string()
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn now_sec() -> i64 {
    now_ms() / 1000
}

fn random_hex32() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stage_roundtrip() {
        let value = parse_stage("review_pending").expect("stage");
        assert_eq!(stage_to_string(value), "review_pending");
    }

    #[test]
    fn random_token_has_32_hex_chars() {
        let token = random_hex32();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
    }
}
