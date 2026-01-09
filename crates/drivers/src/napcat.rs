use std::collections::{HashMap, HashSet};
use std::env;
use std::future::Future;
use std::fs;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::command::{
    GlobalAction, GlobalActionCommand, ReviewAction, ReviewActionCommand,
};
use oqqwall_rust_core::draft::{IngressAttachment, IngressMessage, MediaKind, MediaReference};
use oqqwall_rust_core::event::{
    BlobEvent, DraftEvent, Event, IngressEvent, InputStatusKind, MediaEvent, ReviewDecision,
    ReviewEvent, ScheduleEvent, SendEvent, SendPriority,
};
use oqqwall_rust_core::ids::{BlobId, ExternalCode, IngressId, PostId, ReviewCode, ReviewId};
use oqqwall_rust_core::{derive_blob_id, derive_ingress_id, Command, IngressCommand, StateView};
use oqqwall_rust_infra::{LocalJournal, SnapshotStore};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::Client;
use serde_json::{json, Value};

use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

use crate::blob_cache;

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

#[derive(Debug, Clone)]
pub struct NapCatConfig {
    pub ws_url: String,
    pub access_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NapCatRuntimeConfig {
    pub napcat: NapCatConfig,
    pub audit_group_id: Option<String>,
    pub group_id: String,
    pub tz_offset_minutes: i32,
    pub friend_request_window_sec: u32,
    pub friend_add_message: Option<String>,
    pub max_queue: usize,
}

const MAX_FORWARD_DEPTH: u32 = 4;
const FRIEND_APPROVE_DELAY_MAX_SEC: u64 = 240;
const FRIEND_NOTIFY_DELAY_SEC: u64 = 30;
const FRIEND_REQUEST_ID_MAX_LEN: usize = 20;
const FRIEND_SUPPRESS_REMOVE_CHARS: &str = r#"　“”‘’《》〈〉【】。，：；？！（）、「」『』—［］＂＇"'`~!@#$%^&*()_+-={}[]|:;<>?,./"#;
static STARTUP_NOTICE_SENT: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct ReviewInfo {
    review_code: ReviewCode,
    post_id: PostId,
    group_id: String,
}

#[derive(Debug, Clone)]
struct SendPlanInfo {
    group_id: String,
    not_before_ms: i64,
    priority: SendPriority,
    seq: u64,
}

#[derive(Debug, Clone)]
struct SendingInfo {
    group_id: String,
    started_at_ms: i64,
}

#[derive(Debug, Clone)]
struct IngressSummary {
    user_id: String,
    sender_name: Option<String>,
    text: String,
    attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExtractedMessage {
    pub(crate) text: String,
    pub(crate) summary_text: String,
    pub(crate) attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone)]
struct MessageChunk {
    text: String,
    summary_text: String,
    attachments: Vec<IngressAttachment>,
}

#[derive(Debug)]
struct ForwardResolver {
    client: Client,
    api_base: String,
    token: Option<String>,
    cache: HashMap<String, Vec<MessageChunk>>,
    seen: HashSet<String>,
}

#[derive(Debug, Clone)]
struct AuditMessage {
    text: String,
    images: Vec<String>,
}

#[derive(Debug, Clone)]
enum PendingAction {
    SendAuditMessage { review_id: ReviewId, attempt: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AuditCommand {
    Review {
        review_code: Option<ReviewCode>,
        action: ReviewAction,
    },
    Global(GlobalAction),
}

#[derive(Debug, Clone)]
struct SuppressionEntry {
    comment_norm: String,
    expire_at_ms: i64,
}

#[derive(Default)]
struct NapCatState {
    review_info: HashMap<ReviewId, ReviewInfo>,
    review_by_code: HashMap<ReviewCode, ReviewId>,
    review_publish_attempts: HashMap<ReviewId, u32>,
    ingress_summary: HashMap<IngressId, IngressSummary>,
    pending_summary: HashMap<IngressId, String>,
    post_ingress: HashMap<PostId, Vec<IngressId>>,
    post_group: HashMap<PostId, String>,
    post_safe: HashMap<PostId, bool>,
    post_review_code: HashMap<PostId, ReviewCode>,
    post_external_code: HashMap<PostId, ExternalCode>,
    review_submitter: HashMap<ReviewId, String>,
    blacklist: HashMap<String, HashMap<String, Option<String>>>,
    send_plans: HashMap<PostId, SendPlanInfo>,
    sending: HashMap<PostId, SendingInfo>,
    audit_msg_to_review: HashMap<String, ReviewId>,
    processed_reviews: HashSet<ReviewId>,
    pending: HashMap<String, PendingAction>,
    friend_req_cache: HashMap<String, i64>,
    friend_suppression: HashMap<String, Vec<SuppressionEntry>>,
    blob_paths: HashMap<BlobId, String>,
    next_echo: u64,
}

fn load_state_view_cached() -> StateView {
    static CACHE: OnceLock<StateView> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let data_dir = env::var("OQQWALL_DATA_DIR").unwrap_or_else(|_| "data".to_string());
            let journal = match LocalJournal::open(&data_dir) {
                Ok(journal) => journal,
                Err(_err) => {
                    debug_log!(
                        "napcat preload skipped: journal open failed: {}",
                        _err
                    );
                    return StateView::default();
                }
            };
            let snapshot = match SnapshotStore::open(&data_dir) {
                Ok(snapshot) => snapshot,
                Err(_err) => {
                    debug_log!(
                        "napcat preload skipped: snapshot open failed: {}",
                        _err
                    );
                    return StateView::default();
                }
            };

            let mut state = StateView::default();
            let mut cursor = None;
            match snapshot.load() {
                Ok(Some(loaded)) => {
                    state = loaded.state;
                    cursor = loaded.journal_cursor;
                }
                Ok(None) => {}
                Err(_err) => {
                    debug_log!("napcat preload: snapshot load failed: {}", _err);
                }
            }

            if let Err(_err) = journal.replay(cursor, |env| {
                state = state.reduce(env);
            }) {
                debug_log!("napcat preload: journal replay failed: {}", _err);
            }

            state
        })
        .clone()
}

fn build_state_from_view(view: &StateView) -> NapCatState {
    let mut state = NapCatState::default();
    for (ingress_id, meta) in &view.ingress_meta {
        let (text, attachments) = match view.ingress_messages.get(ingress_id) {
            Some(message) => (message.text.clone(), message.attachments.clone()),
            None => (String::new(), Vec::new()),
        };
        state.ingress_summary.insert(
            *ingress_id,
            IngressSummary {
                user_id: meta.user_id.clone(),
                sender_name: meta.sender_name.clone(),
                text,
                attachments,
            },
        );
    }
    for (post_id, ingress_ids) in &view.post_ingress {
        state.post_ingress.insert(*post_id, ingress_ids.clone());
    }
    for (post_id, post) in &view.posts {
        state.post_group.insert(*post_id, post.group_id.clone());
        state.post_safe.insert(*post_id, post.is_safe);
    }
    for (review_id, review) in &view.reviews {
        let group_id = state
            .post_group
            .get(&review.post_id)
            .cloned()
            .unwrap_or_default();
        state.review_info.insert(
            *review_id,
            ReviewInfo {
                review_code: review.review_code,
                post_id: review.post_id,
                group_id,
            },
        );
        state.review_by_code.insert(review.review_code, *review_id);
        state
            .post_review_code
            .insert(review.post_id, review.review_code);
        if let Some(audit_msg_id) = review.audit_msg_id.as_ref() {
            state
                .audit_msg_to_review
                .insert(audit_msg_id.clone(), *review_id);
        }
        if matches!(
            review.decision,
            Some(
                ReviewDecision::Approved
                    | ReviewDecision::Rejected
                    | ReviewDecision::Skipped
                    | ReviewDecision::Deleted
            )
        ) {
            state.processed_reviews.insert(*review_id);
        }
        if review.publish_attempt > 0 {
            state
                .review_publish_attempts
                .insert(*review_id, review.publish_attempt);
        }
        if let Some(user_id) = resolve_post_submitter(&state, review.post_id) {
            state.review_submitter.insert(*review_id, user_id);
        }
    }
    for (post_id, external_code) in &view.external_code_by_post {
        state.post_external_code.insert(*post_id, *external_code);
    }
    for (group_id, entries) in &view.blacklist {
        state.blacklist.insert(group_id.clone(), entries.clone());
    }
    for (post_id, plan) in &view.send_plans {
        state.send_plans.insert(
            *post_id,
            SendPlanInfo {
                group_id: plan.group_id.clone(),
                not_before_ms: plan.not_before_ms,
                priority: plan.priority,
                seq: plan.seq,
            },
        );
    }
    for (post_id, meta) in &view.sending {
        state.sending.insert(
            *post_id,
            SendingInfo {
                group_id: meta.group_id.clone(),
                started_at_ms: meta.started_at_ms,
            },
        );
    }
    for (blob_id, meta) in &view.blobs {
        if let Some(path) = meta.persisted_path.as_ref() {
            state.blob_paths.insert(*blob_id, path.clone());
        }
    }
    state
}

pub fn spawn_napcat_ws(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    runtime: NapCatRuntimeConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug_log!(
            "napcat ws task start: group_id={} audit_group_id={:?} ws_url={} token_present={}",
            runtime.group_id,
            runtime.audit_group_id,
            ws_url_for_log(&runtime.napcat.ws_url),
            runtime.napcat.access_token.is_some()
        );
        let state_view = load_state_view_cached();
        let state = Arc::new(Mutex::new(build_state_from_view(&state_view)));
        let bus_rx = bus_rx;

        loop {
            let runtime = runtime.clone();
            debug_log!(
                "napcat ws connecting: group_id={} ws_url={}",
                runtime.group_id,
                ws_url_for_log(&runtime.napcat.ws_url)
            );
            let ws_url =
                build_ws_url(&runtime.napcat.ws_url, runtime.napcat.access_token.as_deref());
            let mut request = ws_url.into_client_request().expect("invalid napcat ws url");
            if let Some(token) = runtime.napcat.access_token.as_deref() {
                let header_value = format!("Bearer {}", token);
                if let Ok(value) = header_value.parse() {
                    request.headers_mut().insert("Authorization", value);
                }
            }

            let connect = connect_async(request).await;
            let (ws_stream, _) = match connect {
                Ok(pair) => pair,
                Err(err) => {
                    log_ws_connect_error(&runtime.group_id, &runtime.napcat.ws_url, &err);
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            println!("NapCat WS 已连接: group_id={}", runtime.group_id);

            let (mut ws_write, mut ws_read) = ws_stream.split();
            debug_log!("napcat ws connected: group_id={}", runtime.group_id);
            let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
            let state_ref = Arc::clone(&state);
            let startup_group_id = runtime
                .audit_group_id
                .as_deref()
                .unwrap_or(&runtime.group_id);
            if should_send_startup_notice(startup_group_id) {
                send_group_text(&out_tx, startup_group_id, "系统已启动").await;
            }

            let writer = tokio::spawn(async move {
                while let Some(msg) = out_rx.recv().await {
                    if ws_write
                        .send(tokio_tungstenite::tungstenite::Message::Text(msg))
                        .await
                        .is_err()
                    {
                        debug_log!("napcat ws writer send failed");
                        break;
                    }
                }
            });

            let cmd_tx_read = cmd_tx.clone();
            let runtime_read = runtime.clone();
            let state_read = Arc::clone(&state_ref);
            let out_tx_read = out_tx.clone();
            let reader = tokio::spawn(async move {
                while let Some(msg) = ws_read.next().await {
                    let msg = match msg {
                        Ok(msg) => msg,
                        Err(_err) => {
                            debug_log!("napcat ws read error: {}", _err);
                            break;
                        }
                    };
                    if !msg.is_text() {
                        debug_log!("napcat ws ignoring non-text message");
                        continue;
                    }
                    let text = match msg.to_text() {
                        Ok(text) => text,
                        Err(_err) => {
                            debug_log!("napcat ws text decode error: {}", _err);
                            continue;
                        }
                    };
                    let Ok(value) = serde_json::from_str::<Value>(text) else {
                        debug_log!("napcat ws invalid json: {}", text);
                        continue;
                    };
                    if let Some(echo) = value.get("echo").and_then(|v| v.as_str()) {
                        if let Some(event) =
                            handle_action_response(&state_read, echo, &value).await
                        {
                            debug_log!(
                                "napcat ws action response: echo={} event={:?}",
                                echo,
                                event
                            );
                            let _ = cmd_tx_read.send(Command::DriverEvent(event)).await;
                        }
                        continue;
                    }
                    if let Some(command) = parse_inbound_event(
                        &runtime_read,
                        &state_read,
                        &out_tx_read,
                        &value,
                    )
                    .await
                    {
                        debug_log!("napcat ws inbound command: {:?}", command);
                        let _ = cmd_tx_read.send(command).await;
                    }
                }
            });

            let mut bus_task_rx = bus_rx.resubscribe();
            let state_bus = Arc::clone(&state_ref);
            let runtime_bus = runtime.clone();
            let out_tx_bus = out_tx.clone();
            let bus_task = tokio::spawn(async move {
                loop {
                    let env = match bus_task_rx.recv().await {
                        Ok(env) => env,
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    };

                    let action = build_action_from_event(
                        &runtime_bus,
                        &state_bus,
                        &out_tx_bus,
                        env.event,
                    )
                    .await;
                    if let Some(action) = action {
                        debug_log!(
                            "napcat ws outbound action: group_id={} bytes={}",
                            runtime_bus.group_id,
                            action.len()
                        );
                        let _ = out_tx_bus.send(action).await;
                    }
                }
            });

            let _ = tokio::join!(writer, reader, bus_task);
            debug_log!("napcat ws disconnected: group_id={}", runtime.group_id);
            sleep(Duration::from_secs(2)).await;
        }
    })
}

fn should_send_startup_notice(group_id: &str) -> bool {
    let lock = STARTUP_NOTICE_SENT.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    };
    guard.insert(group_id.to_string())
}

fn build_ws_url(base: &str, token: Option<&str>) -> String {
    if let Some(token) = token {
        if base.contains("?") {
            format!("{}&access_token={}", base, token)
        } else {
            format!("{}?access_token={}", base, token)
        }
    } else {
        base.to_string()
    }
}

fn napcat_http_base(ws_url: &str) -> Option<String> {
    let trimmed = ws_url.split('?').next().unwrap_or(ws_url);
    if let Some(rest) = trimmed.strip_prefix("ws://") {
        let mut base = format!("http://{}", rest.trim_end_matches('/'));
        if base.ends_with("/ws") {
            base = base.trim_end_matches("/ws").trim_end_matches('/').to_string();
        }
        return Some(base);
    }
    if let Some(rest) = trimmed.strip_prefix("wss://") {
        let mut base = format!("https://{}", rest.trim_end_matches('/'));
        if base.ends_with("/ws") {
            base = base.trim_end_matches("/ws").trim_end_matches('/').to_string();
        }
        return Some(base);
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let mut base = trimmed.trim_end_matches('/').to_string();
        if base.ends_with("/ws") {
            base = base.trim_end_matches("/ws").trim_end_matches('/').to_string();
        }
        return Some(base);
    }
    None
}

#[cfg(debug_assertions)]
fn ws_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

fn log_ws_connect_error(
    _group_id: &str,
    _ws_url: &str,
    _err: &tokio_tungstenite::tungstenite::Error,
) {
    match _err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            let _status = response.status();
            let _headers = response.headers();
            let body = response.body().as_ref();
            let _body_len = body.map(|bytes| bytes.len()).unwrap_or(0);
            let _preview = body
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map(|text| text.trim())
                .filter(|text| !text.is_empty())
                .map(|text| {
                    let mut out: String = text.chars().take(256).collect();
                    if text.chars().count() > 256 {
                        out.push_str("...");
                    }
                    out
                });
            if let Some(_preview) = _preview {
                debug_log!(
                    "napcat ws connect failed: group_id={} ws_url={} status={} headers={:?} body_len={} body_preview=\"{}\"",
                    _group_id,
                    ws_url_for_log(_ws_url),
                    _status,
                    _headers,
                    _body_len,
                    _preview
                );
            } else {
                debug_log!(
                    "napcat ws connect failed: group_id={} ws_url={} status={} headers={:?} body_len={}",
                    _group_id,
                    ws_url_for_log(_ws_url),
                    _status,
                    _headers,
                    _body_len
                );
            }
        }
        _ => {
            debug_log!(
                "napcat ws connect failed: group_id={} ws_url={} err={:?}",
                _group_id,
                ws_url_for_log(_ws_url),
                _err
            );
        }
    }
}

async fn build_action_from_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    event: Event,
) -> Option<String> {
    match event {
        Event::Ingress(IngressEvent::InputStatusUpdated { .. }) => None,
        Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id,
            user_id,
            sender_name,
            message,
            ..
        })
        | Event::Ingress(IngressEvent::MessageSynced {
            ingress_id,
            user_id,
            sender_name,
            message,
            ..
        }) => {
            let mut guard = state.lock().await;
            let IngressMessage { text, attachments } = message;
            let summary_text = guard
                .pending_summary
                .remove(&ingress_id)
                .unwrap_or_else(|| text.clone());
            guard.ingress_summary.insert(
                ingress_id,
                IngressSummary {
                    user_id,
                    sender_name,
                    text: summary_text,
                    attachments,
                },
            );
            None
        }
        Event::Ingress(IngressEvent::MessageIgnored { ingress_id, .. }) => {
            let mut guard = state.lock().await;
            guard.pending_summary.remove(&ingress_id);
            None
        }
        Event::Draft(DraftEvent::PostDraftCreated {
            post_id,
            ingress_ids,
            group_id,
            is_safe,
            ..
        }) => {
            let mut guard = state.lock().await;
            guard.post_ingress.insert(post_id, ingress_ids);
            guard.post_group.insert(post_id, group_id);
            guard.post_safe.insert(post_id, is_safe);
            None
        }
        Event::Review(ReviewEvent::ReviewInfoSynced {
            review_id,
            post_id,
            review_code,
        }) => {
            let mut guard = state.lock().await;
            let group_id = guard
                .post_group
                .get(&post_id)
                .cloned()
                .unwrap_or_default();
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                    group_id,
                },
            );
            guard.review_by_code.insert(review_code, review_id);
            guard.post_review_code.insert(post_id, review_code);
            if let Some(user_id) = resolve_post_submitter(&guard, post_id) {
                guard.review_submitter.insert(review_id, user_id);
            }
            None
        }
        Event::Media(MediaEvent::MediaFetchSucceeded {
            ingress_id,
            attachment_index,
            blob_id,
        }) => {
            let mut guard = state.lock().await;
            if let Some(summary) = guard.ingress_summary.get_mut(&ingress_id) {
                if let Some(attachment) = summary.attachments.get_mut(attachment_index) {
                    attachment.reference = MediaReference::Blob { blob_id };
                }
            }
            None
        }
        Event::Blob(BlobEvent::BlobPersisted { blob_id, path }) => {
            let mut guard = state.lock().await;
            guard.blob_paths.insert(blob_id, path.clone());
            None
        }
        Event::Blob(BlobEvent::BlobReleased { blob_id })
        | Event::Blob(BlobEvent::BlobGcRequested { blob_id }) => {
            let mut guard = state.lock().await;
            guard.blob_paths.remove(&blob_id);
            None
        }
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        }) => {
            debug_log!(
                "napcat review created: review_id={} post_id={} review_code={}",
                review_id.0,
                post_id.0,
                review_code
            );
            let mut guard = state.lock().await;
            let group_id = guard
                .post_group
                .get(&post_id)
                .cloned()
                .unwrap_or_default();
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                    group_id,
                },
            );
            guard.review_by_code.insert(review_code, review_id);
            guard.post_review_code.insert(post_id, review_code);
            if let Some(user_id) = resolve_post_submitter(&guard, post_id) {
                guard.review_submitter.insert(review_id, user_id);
            }
            None
        }
        Event::Review(ReviewEvent::ReviewExternalCodeAssigned {
            post_id,
            external_code,
            ..
        }) => {
            let mut guard = state.lock().await;
            guard.post_external_code.insert(post_id, external_code);
            None
        }
        Event::Review(ReviewEvent::ReviewPublished {
            review_id,
            audit_msg_id,
        }) => {
            debug_log!(
                "napcat review published: review_id={} audit_msg_id={}",
                review_id.0,
                audit_msg_id
            );
            let mut guard = state.lock().await;
            guard.audit_msg_to_review.insert(audit_msg_id, review_id);
            guard.review_publish_attempts.remove(&review_id);
            None
        }
        Event::Review(ReviewEvent::ReviewPublishFailed {
            review_id,
            attempt,
            error: _error,
            ..
        }) => {
            debug_log!(
                "napcat review publish failed: review_id={} attempt={} err={}",
                review_id.0,
                attempt,
                _error
            );
            let mut guard = state.lock().await;
            guard.review_publish_attempts.insert(review_id, attempt);
            None
        }
        Event::Review(ReviewEvent::ReviewDecisionRecorded {
            review_id,
            decision,
            ..
        }) => {
            let should_notify = matches!(decision, ReviewDecision::Rejected);
            let submitter = {
                let mut guard = state.lock().await;
                match decision {
                    ReviewDecision::Approved
                    | ReviewDecision::Rejected
                    | ReviewDecision::Skipped
                    | ReviewDecision::Deleted => {
                        guard.processed_reviews.insert(review_id);
                    }
                    ReviewDecision::Deferred => {
                        guard.processed_reviews.remove(&review_id);
                    }
                }
                if should_notify {
                    resolve_review_submitter(&guard, review_id)
                } else {
                    None
                }
            };
            if !should_notify {
                return None;
            }
            let Some((group_id, user_id)) = submitter else {
                debug_log!("napcat reject notify skipped: missing submitter info");
                return None;
            };
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            let text = "你的投稿已被拒，请修改后再发送";
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewReplyRequested { review_id, text }) => {
            if text.trim().is_empty() {
                debug_log!("napcat reply skipped: empty text");
                return None;
            }
            let submitter = {
                let guard = state.lock().await;
                resolve_review_submitter(&guard, review_id)
            };
            let Some((group_id, user_id)) = submitter else {
                debug_log!("napcat reply skipped: missing submitter info");
                return None;
            };
            if !group_id.is_empty() && group_id != runtime.group_id {
                return None;
            }
            let payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewBlacklistRequested { review_id, reason }) => {
            let mut guard = state.lock().await;
            let Some((group_id, sender_id)) = resolve_review_submitter(&guard, review_id) else {
                debug_log!("napcat blacklist skipped: missing review submitter");
                return None;
            };
            let entry = guard
                .blacklist
                .entry(group_id)
                .or_default()
                .entry(sender_id)
                .or_insert(None);
            if reason.is_some() {
                *entry = reason.clone();
            }
            None
        }
        Event::Review(ReviewEvent::ReviewBlacklistRemoved {
            group_id,
            sender_id,
        }) => {
            let mut guard = state.lock().await;
            if let Some(group) = guard.blacklist.get_mut(&group_id) {
                group.remove(&sender_id);
                if group.is_empty() {
                    guard.blacklist.remove(&group_id);
                }
            }
            None
        }
        Event::Schedule(ScheduleEvent::SendPlanCreated {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        }) => {
            let stacking_enabled = runtime.max_queue > 1;
            let (label, label_plain, code_text, submitter_id, should_notify, audit_group_id) = {
                let mut guard = state.lock().await;
                guard.send_plans.insert(
                    post_id,
                    SendPlanInfo {
                        group_id: group_id.clone(),
                        not_before_ms,
                        priority,
                        seq,
                    },
                );
                (
                    post_label(&guard, post_id),
                    post_label_plain(&guard, post_id),
                    post_code_text(&guard, post_id),
                    resolve_post_submitter(&guard, post_id),
                    group_id == runtime.group_id,
                    runtime.audit_group_id.clone(),
                )
            };
            if !should_notify {
                return None;
            }
            let Some(audit_group_id) = audit_group_id else {
                return None;
            };
            if stacking_enabled {
                if let (Some(code), Some(user_id)) = (code_text, submitter_id) {
                    let text = format!("#{}已通过审核,待发送", code);
                    let payload = serde_json::json!({
                        "action": "send_private_msg",
                        "params": {
                            "user_id": json_id(&user_id),
                            "message": message_segments_from_text(&text)
                        }
                    });
                    let _ = out_tx.send(payload.to_string()).await;
                }
                let text = format!("{}已存入暂存区", label_plain);
                let payload = serde_json::json!({
                    "action": "send_group_msg",
                    "params": {
                        "group_id": json_id(&audit_group_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                return Some(payload.to_string());
            }
            let text = format!("{}正在发送...", label);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(&audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Schedule(ScheduleEvent::SendPlanRescheduled {
            post_id,
            group_id,
            not_before_ms,
            priority,
            seq,
        }) => {
            let mut guard = state.lock().await;
            guard.send_plans.insert(
                post_id,
                SendPlanInfo {
                    group_id,
                    not_before_ms,
                    priority,
                    seq,
                },
            );
            None
        }
        Event::Schedule(ScheduleEvent::SendPlanCanceled { post_id }) => {
            let mut guard = state.lock().await;
            guard.send_plans.remove(&post_id);
            None
        }
        Event::Send(SendEvent::SendStarted {
            post_id,
            group_id,
            started_at_ms,
            ..
        }) => {
            let stacking_enabled = runtime.max_queue > 1;
            let (label_plain, should_notify, audit_group_id) = {
                let mut guard = state.lock().await;
                guard.send_plans.remove(&post_id);
                guard.sending.insert(
                    post_id,
                    SendingInfo {
                        group_id: group_id.clone(),
                        started_at_ms,
                    },
                );
                (
                    post_label_plain(&guard, post_id),
                    group_id == runtime.group_id,
                    runtime.audit_group_id.clone(),
                )
            };
            if !stacking_enabled || !should_notify {
                return None;
            }
            let Some(audit_group_id) = audit_group_id else {
                return None;
            };
            let text = format!("{}正在发送中", label_plain);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(&audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendSucceeded {
            post_id,
            account_id,
            ..
        }) => {
            let (group_id, audit_group_id, send_code, submitter_id) = {
                let mut guard = state.lock().await;
                let group_id = guard
                    .sending
                    .remove(&post_id)
                    .map(|info| info.group_id)
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_else(|| runtime.group_id.clone());
                let send_code = post_code_text(&guard, post_id);
                let submitter_id = resolve_post_submitter(&guard, post_id);
                (
                    group_id,
                    runtime.audit_group_id.clone(),
                    send_code,
                    submitter_id,
                )
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            if let (Some(code), Some(user_id)) = (send_code, submitter_id) {
                let text = format!("#{}已发送", code);
                let payload = serde_json::json!({
                    "action": "send_private_msg",
                    "params": {
                        "user_id": json_id(&user_id),
                        "message": message_segments_from_text(&text)
                    }
                });
                let _ = out_tx.send(payload.to_string()).await;
            }
            let Some(audit_group_id) = audit_group_id else {
                return None;
            };
            let text = format!("{}已发送", account_id);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(&audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendFailed {
            post_id,
            account_id,
            attempt,
            error,
            ..
        }) => {
            let (group_id, label) = {
                let mut guard = state.lock().await;
                let group_id = guard
                    .sending
                    .remove(&post_id)
                    .map(|info| info.group_id)
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_default();
                let label = post_label(&guard, post_id);
                (group_id, label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            if !is_send_timeout_error(&error) {
                return None;
            }
            let text = format!(
                "{} 发送超时（账号{} 第{}次）：{}",
                label, account_id, attempt, error
            );
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Send(SendEvent::SendGaveUp { post_id, reason }) => {
            let (group_id, label) = {
                let mut guard = state.lock().await;
                let group_id = guard
                    .sending
                    .remove(&post_id)
                    .map(|info| info.group_id)
                    .or_else(|| guard.post_group.get(&post_id).cloned())
                    .unwrap_or_default();
                let label = post_label(&guard, post_id);
                (group_id, label)
            };
            if group_id.is_empty() || group_id != runtime.group_id {
                return None;
            }
            let Some(audit_group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let text = format!("{} 发送失败已停止重试：{}", label, reason);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(audit_group_id),
                    "message": message_segments_from_text(&text)
                }
            });
            Some(payload.to_string())
        }
        Event::Review(ReviewEvent::ReviewPublishRequested { review_id }) => {
            let Some(group_id) = runtime.audit_group_id.as_ref() else {
                return None;
            };
            let mut guard = state.lock().await;
            let Some(info) = guard.review_info.get(&review_id).cloned() else {
                debug_log!("napcat review publish requested but missing review info");
                return None;
            };
            let attempt = {
                let entry = guard
                    .review_publish_attempts
                    .entry(review_id)
                    .and_modify(|value| *value = value.saturating_add(1))
                    .or_insert(1);
                *entry
            };
            let ingress_ids = guard
                .post_ingress
                .get(&info.post_id)
                .cloned()
                .unwrap_or_default();
            if let Some(user_id) = resolve_post_submitter_with_ingress(&guard, &ingress_ids) {
                guard.review_submitter.insert(review_id, user_id);
            }
            let preview = rendered_png_preview(info.post_id);
            let is_safe = guard
                .post_safe
                .get(&info.post_id)
                .copied()
                .unwrap_or(true);
            let summary = build_audit_message(
                info.review_code,
                info.post_id,
                &ingress_ids,
                &guard.ingress_summary,
                preview,
                &guard.blob_paths,
                is_safe,
            );
            let echo = next_echo(&mut guard);
            guard.pending.insert(
                echo.clone(),
                PendingAction::SendAuditMessage {
                    review_id,
                    attempt,
                },
            );

            let mut message = message_segments_from_text(&summary.text);
            for image in summary.images {
                message.push(serde_json::json!({
                    "type": "image",
                    "data": { "file": image }
                }));
            }
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": json_id(group_id),
                    "message": message
                },
                "echo": echo
            });
            Some(payload.to_string())
        }
        _ => None,
    }
}

async fn handle_action_response(
    state: &Arc<Mutex<NapCatState>>,
    echo: &str,
    value: &Value,
) -> Option<Event> {
    let mut guard = state.lock().await;
    let pending = guard.pending.remove(echo)?;
    // OneBot/NapCat action responses look like:
    // {"status":"ok","retcode":0,"data":{...},"echo":"..."}
    // If failed (e.g. wrong group_id type/permission issues), data may be empty.
    let status = value
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let retcode = value.get("retcode").and_then(|v| v.as_i64()).unwrap_or(-1);
    match pending {
        PendingAction::SendAuditMessage { review_id, attempt } => {
            if status != "ok" || retcode != 0 {
                debug_log!(
                    "napcat action failed: echo={} status={} retcode={} raw={}",
                    echo,
                    status,
                    retcode,
                    value
                );
                let msg = value
                    .get("msg")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let mut error = format!("action failed status={} retcode={}", status, retcode);
                if !msg.is_empty() {
                    error.push_str(&format!(" msg={}", msg));
                }
                let retry_at_ms =
                    now_ms().saturating_add(review_retry_delay_ms(attempt));
                return Some(Event::Review(ReviewEvent::ReviewPublishFailed {
                    review_id,
                    attempt,
                    retry_at_ms,
                    error,
                }));
            }

            let message_id = value
                .get("data")
                .and_then(|data| data.get("message_id"))
                .and_then(value_to_string);
            let Some(message_id) = message_id else {
                let retry_at_ms =
                    now_ms().saturating_add(review_retry_delay_ms(attempt));
                return Some(Event::Review(ReviewEvent::ReviewPublishFailed {
                    review_id,
                    attempt,
                    retry_at_ms,
                    error: "missing message_id in action response".to_string(),
                }));
            };
            debug_log!(
                "napcat audit message sent: review_id={} message_id={}",
                review_id.0,
                message_id
            );
            guard
                .audit_msg_to_review
                .insert(message_id.clone(), review_id);
            Some(Event::Review(ReviewEvent::ReviewPublished {
                review_id,
                audit_msg_id: message_id,
            }))
        }
    }
}

fn json_id(id: &str) -> Value {
    let trimmed = id.trim();
    if let Ok(n) = trimmed.parse::<i64>() {
        Value::Number(n.into())
    } else {
        Value::String(trimmed.to_string())
    }
}

fn is_send_timeout_error(error: &str) -> bool {
    error.starts_with("send timeout")
}

fn review_retry_delay_ms(attempt: u32) -> i64 {
    let base = 5_000i64;
    let max = 60_000i64;
    let shift = attempt.saturating_sub(1).min(10);
    let delay = base.saturating_mul(1_i64 << shift);
    delay.min(max)
}

async fn parse_inbound_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    value: &Value,
) -> Option<Command> {
    let post_type = value.get("post_type").and_then(|v| v.as_str())?;
    if post_type == "notice" {
        return parse_notice_event(runtime, value);
    }
    if post_type == "request" {
        handle_friend_request(runtime, state, out_tx, value).await;
        return None;
    }
    if post_type != "message" && post_type != "message_sent" {
        return None;
    }

    let message_type = value.get("message_type").and_then(|v| v.as_str())?;
    debug_log!(
        "napcat inbound: post_type={} message_type={}",
        post_type,
        message_type
    );
    let user_id = value_opt_to_string(value.get("user_id"))?;
    let self_id =
        value_opt_to_string(value.get("self_id")).unwrap_or_else(|| "napcat".to_string());
    let message_id =
        value_opt_to_string(value.get("message_id")).unwrap_or_else(|| "0".to_string());
    let sender_name = extract_sender_name(value);
    let timestamp_ms = inbound_timestamp_ms(value);

    if message_type == "private" && (post_type == "message_sent" || user_id == self_id) {
        debug_log!(
            "napcat inbound ignored private sent/self message: post_type={} user_id={} self_id={}",
            post_type,
            user_id,
            self_id
        );
        return None;
    }

    if message_type == "group" {
        let mut forward_resolver = if message_has_forward(value.get("message")) {
            napcat_http_base(&runtime.napcat.ws_url).and_then(|api_base| {
                Client::builder()
                    .timeout(Duration::from_secs(6))
                    .build()
                    .ok()
                    .map(|client| ForwardResolver {
                        client,
                        api_base,
                        token: runtime.napcat.access_token.clone(),
                        cache: HashMap::new(),
                        seen: HashSet::new(),
                    })
            })
        } else {
            None
        };
        let (extracted, reply_id) =
            extract_message(value.get("message"), &mut forward_resolver).await;
        let ExtractedMessage {
            text,
            summary_text: _,
            attachments: _attachments,
        } = extracted;
        debug_log!(
            "napcat inbound content: text_len={} attachments={} reply_id_present={}",
            text.len(),
            _attachments.len(),
            reply_id.is_some()
        );
        let chat_group_id = value_opt_to_string(value.get("group_id"))?;
        let is_audit_group =
            runtime.audit_group_id.as_deref() == Some(chat_group_id.as_str());
        if let Some(command) = parse_audit_command(&text, reply_id.is_some()) {
            if !is_admin_sender(value) {
                send_group_text(out_tx, &chat_group_id, "无权限执行指令").await;
                return None;
            }
            if runtime.audit_group_id.is_some() && !is_audit_group {
                return None;
            }
            match command {
                AuditCommand::Global(GlobalAction::Help) => {
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    send_group_text(out_tx, &chat_group_id, HELP_TEXT).await;
                    return None;
                }
                AuditCommand::Global(GlobalAction::PendingList) => {
                    let pending_text = {
                        let guard = state.lock().await;
                        build_pending_list_text(&guard, &runtime.group_id)
                    };
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    send_group_text(out_tx, &chat_group_id, &pending_text).await;
                    return None;
                }
                AuditCommand::Global(GlobalAction::BlacklistList) => {
                    let blacklist_text = {
                        let guard = state.lock().await;
                        build_blacklist_list_text(&guard, &runtime.group_id)
                    };
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    send_group_text(out_tx, &chat_group_id, &blacklist_text).await;
                    return None;
                }
                AuditCommand::Global(GlobalAction::BlacklistRemove { sender_id }) => {
                    let removed = {
                        let mut guard = state.lock().await;
                        if let Some(group) = guard.blacklist.get_mut(&runtime.group_id) {
                            let removed = group.remove(&sender_id).is_some();
                            if group.is_empty() {
                                guard.blacklist.remove(&runtime.group_id);
                            }
                            removed
                        } else {
                            false
                        }
                    };
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    let text = if removed {
                        format!("已取消拉黑 {}", sender_id)
                    } else {
                        format!("黑名单中不存在 {}", sender_id)
                    };
                    send_group_text(out_tx, &chat_group_id, &text).await;
                    return Some(Command::GlobalAction(GlobalActionCommand {
                        group_id: runtime.group_id.clone(),
                        action: GlobalAction::BlacklistRemove { sender_id },
                        operator_id: user_id.to_string(),
                        now_ms: timestamp_ms,
                        tz_offset_minutes: runtime.tz_offset_minutes,
                    }));
                }
                AuditCommand::Global(action) => {
                    if let GlobalAction::Recall { review_code } = &action {
                        let mut guard = state.lock().await;
                        if let Some(review_id) = guard.review_by_code.get(review_code).copied() {
                            guard.processed_reviews.remove(&review_id);
                        }
                    }
                    send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                    return Some(Command::GlobalAction(GlobalActionCommand {
                        group_id: runtime.group_id.clone(),
                        action,
                        operator_id: user_id.to_string(),
                        now_ms: timestamp_ms,
                        tz_offset_minutes: runtime.tz_offset_minutes,
                    }));
                }
                AuditCommand::Review { review_code, action } => {
                    return parse_review_command(
                        runtime,
                        state,
                        out_tx,
                        &user_id,
                        &chat_group_id,
                        review_code,
                        action,
                        reply_id,
                        timestamp_ms,
                    )
                    .await;
                }
            }
        }
        if is_audit_group {
            return None;
        }
        debug_log!(
            "napcat inbound ignored group message for ingress: group_id={}",
            chat_group_id
        );
        return None;
    }

    if message_type == "private" {
        let raw_message = value.get("raw_message").and_then(|v| v.as_str());
        if let Some(raw_message) = raw_message {
            if is_auto_reply_message(raw_message) {
                debug_log!("napcat inbound ignored private system message");
                return None;
            }
        }
        let ExtractedMessage {
            text,
            summary_text,
            attachments,
        } = extract_message_lite(value.get("message"));
        if raw_message
            .map(|raw| raw.is_empty())
            .unwrap_or(true)
            && is_auto_reply_message(&summary_text)
        {
            debug_log!("napcat inbound ignored private system message");
            return None;
        }
        debug_log!(
            "napcat inbound private lite: text_len={} attachments={}",
            text.len(),
            attachments.len()
        );
        let ingress_id = derive_ingress_id(&[
            self_id.as_bytes(),
            user_id.as_bytes(),
            user_id.as_bytes(),
            message_id.as_bytes(),
        ]);
        let suppress_text = match raw_message {
            Some(raw) if !raw.is_empty() => raw,
            _ => summary_text.as_str(),
        };
        {
            let mut guard = state.lock().await;
            if should_suppress_private_message(
                &mut guard.friend_suppression,
                &user_id,
                suppress_text,
                now_ms(),
            ) {
                debug_log!("napcat inbound private suppressed after friend request");
                return None;
            }
            guard.pending_summary.insert(ingress_id, summary_text);
        }
        return Some(Command::Ingress(IngressCommand {
            profile_id: self_id,
            chat_id: user_id.clone(),
            user_id,
            sender_name,
            group_id: runtime.group_id.clone(),
            platform_msg_id: message_id,
            message: IngressMessage { text, attachments },
            received_at_ms: timestamp_ms,
        }));
    }

    None
}

async fn handle_friend_request(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    value: &Value,
) {
    let request_type = value.get("request_type").and_then(|v| v.as_str());
    if request_type != Some("friend") {
        return;
    }

    let user_id = value_opt_to_string(value.get("user_id")).unwrap_or_default();
    let flag = value_opt_to_string(value.get("flag")).unwrap_or_default();
    let self_id = value_opt_to_string(value.get("self_id")).unwrap_or_default();
    let comment = value
        .get("comment")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if !is_digits(&user_id, FRIEND_REQUEST_ID_MAX_LEN)
        || !is_digits(&flag, FRIEND_REQUEST_ID_MAX_LEN)
        || !is_digits(&self_id, FRIEND_REQUEST_ID_MAX_LEN)
    {
        debug_log!(
            "napcat friend request ignored: invalid identifiers user_id={} flag={} self_id={}",
            user_id,
            flag,
            self_id
        );
        return;
    }

    let window_ms = runtime
        .friend_request_window_sec
        .saturating_mul(1000) as i64;
    if window_ms > 0 {
        let now_ms = now_ms();
        let mut guard = state.lock().await;
        if !should_process_friend_request(
            &mut guard.friend_req_cache,
            &user_id,
            now_ms,
            window_ms,
        ) {
            debug_log!(
                "napcat friend request ignored: duplicate user_id={} window_sec={}",
                user_id,
                runtime.friend_request_window_sec
            );
            return;
        }
        if !comment.is_empty() {
            add_friend_request_suppression(
                &mut guard.friend_suppression,
                &user_id,
                &comment,
                now_ms,
                window_ms,
            );
        }
    }

    let approve_delay_sec = friend_request_delay_sec();
    let friend_add_message = runtime
        .friend_add_message
        .clone()
        .and_then(|msg| if msg.trim().is_empty() { None } else { Some(msg) });
    let out_tx = out_tx.clone();
    tokio::spawn(async move {
        if approve_delay_sec > 0 {
            sleep(Duration::from_secs(approve_delay_sec)).await;
        }
        let approve_payload = serde_json::json!({
            "action": "set_friend_add_request",
            "params": {
                "flag": flag,
                "approve": true
            }
        });
        let _ = out_tx.send(approve_payload.to_string()).await;
        if let Some(text) = friend_add_message {
            sleep(Duration::from_secs(FRIEND_NOTIFY_DELAY_SEC)).await;
            let message_payload = serde_json::json!({
                "action": "send_private_msg",
                "params": {
                    "user_id": json_id(&user_id),
                    "message": message_segments_from_text(&text)
                }
            });
            let _ = out_tx.send(message_payload.to_string()).await;
        }
    });
}

fn is_auto_reply_message(text: &str) -> bool {
    text.contains("自动回复")
        || text.contains("请求添加你为好友")
        || text.contains("我们已成功添加为好友")
}

fn is_digits(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.chars().all(|ch| ch.is_ascii_digit())
}

fn is_digits_unbounded(value: &str) -> bool {
    is_digits(value, usize::MAX)
}

fn normalize_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        if FRIEND_SUPPRESS_REMOVE_CHARS.contains(ch) {
            continue;
        }
        out.push(ch);
    }
    out
}

fn should_process_friend_request(
    cache: &mut HashMap<String, i64>,
    user_id: &str,
    now_ms: i64,
    window_ms: i64,
) -> bool {
    if user_id.is_empty() || window_ms <= 0 {
        return true;
    }
    cache.retain(|_, exp| *exp > now_ms);
    if let Some(expire_at) = cache.get(user_id) {
        if *expire_at > now_ms {
            return false;
        }
    }
    cache.insert(user_id.to_string(), now_ms.saturating_add(window_ms));
    true
}

fn add_friend_request_suppression(
    cache: &mut HashMap<String, Vec<SuppressionEntry>>,
    user_id: &str,
    comment: &str,
    now_ms: i64,
    window_ms: i64,
) {
    if user_id.is_empty() || comment.is_empty() || window_ms <= 0 {
        return;
    }
    let normalized = normalize_text(comment);
    if normalized.is_empty() {
        return;
    }
    let entry = SuppressionEntry {
        comment_norm: normalized,
        expire_at_ms: now_ms.saturating_add(window_ms),
    };
    let list = cache.entry(user_id.to_string()).or_default();
    list.push(entry);
    list.retain(|item| item.expire_at_ms > now_ms);
}

fn should_suppress_private_message(
    cache: &mut HashMap<String, Vec<SuppressionEntry>>,
    user_id: &str,
    text: &str,
    now_ms: i64,
) -> bool {
    if user_id.is_empty() || text.is_empty() {
        return false;
    }
    let normalized = normalize_text(text);
    if normalized.is_empty() {
        return false;
    }
    let Some(list) = cache.get_mut(user_id) else {
        return false;
    };
    list.retain(|item| item.expire_at_ms > now_ms);
    list.iter().any(|item| item.comment_norm == normalized)
}

fn friend_request_delay_sec() -> u64 {
    if FRIEND_APPROVE_DELAY_MAX_SEC == 0 {
        return 0;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    (nanos % (FRIEND_APPROVE_DELAY_MAX_SEC as u128 + 1)) as u64
}

fn parse_notice_event(runtime: &NapCatRuntimeConfig, value: &Value) -> Option<Command> {
    let notice_type = value.get("notice_type").and_then(|v| v.as_str());
    let sub_type = value.get("sub_type").and_then(|v| v.as_str());
    let is_input_status = (matches!(notice_type, Some("notify"))
        && matches!(sub_type, Some("input_status")))
        || matches!(notice_type, Some("input_status"))
        || matches!(sub_type, Some("input_status"));
    if !is_input_status {
        return None;
    }

    let user_id = value_opt_to_string(value.get("user_id"))
        .or_else(|| value.get("data").and_then(|data| value_opt_to_string(data.get("user_id"))))?;
    let status_raw = value_opt_to_u8(value.get("event_type"))
        .or_else(|| value.get("status").and_then(|status| value_opt_to_u8(status.get("event_type"))))
        .or_else(|| value.get("data").and_then(|data| value_opt_to_u8(data.get("event_type"))))?;
    let status = match status_raw {
        0 => InputStatusKind::Speaking,
        1 => InputStatusKind::Typing,
        2 => InputStatusKind::Stopped,
        other => InputStatusKind::Unknown(other),
    };
    let profile_id =
        value_opt_to_string(value.get("self_id")).unwrap_or_else(|| "napcat".to_string());
    let timestamp_ms = inbound_timestamp_ms(value);

    Some(Command::DriverEvent(Event::Ingress(
        IngressEvent::InputStatusUpdated {
            profile_id,
            chat_id: user_id.clone(),
            user_id,
            group_id: runtime.group_id.clone(),
            status,
            received_at_ms: timestamp_ms,
        },
    )))
}

fn message_has_forward(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => items.iter().any(|item| {
            item.get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|kind| kind == "forward")
        }),
        _ => false,
    }
}

fn forward_placeholder(id: &str) -> String {
    if id.is_empty() {
        "[合并转发]".to_string()
    } else {
        format!("[合并转发:{}]", id)
    }
}

fn push_chunk(
    chunks: &mut Vec<MessageChunk>,
    text: &mut String,
    summary_text: &mut String,
    attachments: &mut Vec<IngressAttachment>,
) {
    let text_value = text.trim().to_string();
    let summary_value = summary_text.trim().to_string();
    let attachments_value = std::mem::take(attachments);
    if !text_value.is_empty() || !summary_value.is_empty() || !attachments_value.is_empty() {
        chunks.push(MessageChunk {
            text: text_value,
            summary_text: summary_value,
            attachments: attachments_value,
        });
    }
    text.clear();
    summary_text.clear();
}

fn extract_message_chunks<'a>(
    value: Option<&'a Value>,
    mut resolver: Option<&'a mut ForwardResolver>,
    depth: u32,
    capture_reply: bool,
) -> Pin<Box<dyn Future<Output = (Vec<MessageChunk>, Option<String>)> + Send + 'a>> {
    Box::pin(async move {
        let mut chunks = Vec::new();
        let mut text = String::new();
        let mut summary_text = String::new();
        let mut attachments = Vec::new();
        let mut reply_id = None;

        match value {
            Some(Value::String(s)) => {
                let extracted = extract_cq_faces(s);
                text.push_str(&extracted);
                summary_text.push_str(&extracted);
            }
            Some(Value::Array(items)) => {
                for item in items {
                    let segment_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let data = item.get("data");
                    match segment_type {
                        "text" => {
                            if let Some(segment) =
                                data.and_then(|d| d.get("text")).and_then(|v| v.as_str())
                            {
                                text.push_str(segment);
                                summary_text.push_str(segment);
                            }
                        }
                        "reply" => {
                            if capture_reply {
                                if let Some(id) = data
                                    .and_then(|d| d.get("id"))
                                    .and_then(value_to_string)
                                {
                                    reply_id = Some(id);
                                }
                            }
                        }
                        "face" => {
                            if let Some(id) = data
                                .and_then(|d| d.get("id"))
                                .and_then(value_to_string)
                            {
                                let placeholder = face_inline_placeholder(&id)
                                    .unwrap_or_else(|| format!("[face:{}]", id));
                                text.push_str(&placeholder);
                                summary_text.push_str(&placeholder);
                            }
                        }
                        "forward" => {
                            let id = data
                                .and_then(|d| d.get("id"))
                                .and_then(value_to_string)
                                .unwrap_or_default();
                            push_chunk(&mut chunks, &mut text, &mut summary_text, &mut attachments);
                            if let Some(resolver) = resolver.as_mut() {
                                let mut resolved =
                                    resolve_forward_chunks(&id, resolver, depth).await;
                                chunks.append(&mut resolved);
                            } else {
                                let placeholder = forward_placeholder(&id);
                                chunks.push(MessageChunk {
                                    text: placeholder.clone(),
                                    summary_text: placeholder,
                                    attachments: Vec::new(),
                                });
                            }
                        }
                        "image" => {
                            let kind = image_kind_from_data(data);
                            if let Some(reference) = extract_reference(data) {
                                attachments.push(IngressAttachment {
                                    kind,
                                    name: data
                                        .and_then(|d| d.get("name"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                    reference,
                                    size_bytes: extract_attachment_size(data),
                                });
                            } else {
                                summary_text.push_str(attachment_placeholder(kind));
                            }
                        }
                        "video" | "file" | "record" => {
                            if let Some(reference) = extract_reference(data) {
                                attachments.push(IngressAttachment {
                                    kind: match segment_type {
                                        "video" => MediaKind::Video,
                                        "file" => MediaKind::File,
                                        "record" => MediaKind::Audio,
                                        _ => MediaKind::Other,
                                    },
                                    name: data
                                        .and_then(|d| d.get("name"))
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                    reference,
                                    size_bytes: extract_attachment_size(data),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        push_chunk(&mut chunks, &mut text, &mut summary_text, &mut attachments);
        (chunks, reply_id)
    })
}

async fn resolve_forward_chunks(
    forward_id: &str,
    resolver: &mut ForwardResolver,
    depth: u32,
) -> Vec<MessageChunk> {
    if forward_id.is_empty() || depth >= MAX_FORWARD_DEPTH {
        let placeholder = forward_placeholder(forward_id);
        return vec![MessageChunk {
            text: placeholder.clone(),
            summary_text: placeholder,
            attachments: Vec::new(),
        }];
    }

    if let Some(cached) = resolver.cache.get(forward_id) {
        return cached.clone();
    }
    if resolver.seen.contains(forward_id) {
        let placeholder = forward_placeholder(forward_id);
        return vec![MessageChunk {
            text: placeholder.clone(),
            summary_text: placeholder,
            attachments: Vec::new(),
        }];
    }
    resolver.seen.insert(forward_id.to_string());

    let resolved = match fetch_forward_messages(resolver, forward_id).await {
        Ok(messages) => forward_messages_to_chunks(&messages, resolver, depth + 1).await,
        Err(_err) => {
            debug_log!("forward resolve failed: id={} err={}", forward_id, _err);
            let placeholder = forward_placeholder(forward_id);
            vec![MessageChunk {
                text: placeholder.clone(),
                summary_text: placeholder,
                attachments: Vec::new(),
            }]
        }
    };
    resolver.cache.insert(forward_id.to_string(), resolved.clone());
    resolved
}

async fn fetch_forward_messages(
    resolver: &ForwardResolver,
    forward_id: &str,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/get_forward_msg", resolver.api_base);
    let mut req = resolver
        .client
        .post(url)
        .json(&json!({ "message_id": forward_id }));
    if let Some(token) = resolver.token.as_ref() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req.send().await.map_err(|err| format!("http error: {}", err))?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|err| format!("json error: {}", err))?;
    if !status.is_success() {
        return Err(format!("http status {}", status));
    }
    if body.get("status").and_then(|v| v.as_str()) != Some("ok") {
        return Err(format!("napcat status {:?}", body.get("status")));
    }
    let messages = body
        .get("data")
        .and_then(|v| v.get("messages"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing forward messages".to_string())?;
    Ok(messages.to_vec())
}

async fn forward_messages_to_chunks(
    messages: &[Value],
    resolver: &mut ForwardResolver,
    depth: u32,
) -> Vec<MessageChunk> {
    let mut chunks = Vec::new();
    for message in messages {
        let payload = message.get("message").or_else(|| message.get("content"));
        let (mut msg_chunks, _) =
            extract_message_chunks(payload, Some(&mut *resolver), depth, false).await;
        chunks.append(&mut msg_chunks);
    }
    chunks
}

async fn extract_message(
    value: Option<&Value>,
    resolver: &mut Option<ForwardResolver>,
) -> (ExtractedMessage, Option<String>) {
    let (chunks, reply_id) = extract_message_chunks(value, resolver.as_mut(), 0, true).await;
    let mut parts = Vec::new();
    let mut summary_parts = Vec::new();
    let mut attachments = Vec::new();
    for chunk in chunks {
        if !chunk.text.is_empty() {
            parts.push(chunk.text);
        }
        if !chunk.summary_text.is_empty() {
            summary_parts.push(chunk.summary_text);
        }
        attachments.extend(chunk.attachments);
    }
    let text = parts.join("\n\n");
    let summary_text = summary_parts.join("\n\n");
    (
        ExtractedMessage {
            text: text.trim().to_string(),
            summary_text: summary_text.trim().to_string(),
            attachments,
        },
        reply_id,
    )
}

pub(crate) fn extract_message_lite(value: Option<&Value>) -> ExtractedMessage {
    let mut text = String::new();
    let mut summary_text = String::new();
    let mut attachments = Vec::new();

    match value {
        Some(Value::String(s)) => {
            let extracted = extract_cq_faces(s);
            text.push_str(&extracted);
            summary_text.push_str(&extracted);
        }
        Some(Value::Array(items)) => {
            for item in items {
                let segment_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let data = item.get("data");
                match segment_type {
                    "text" => {
                        if let Some(segment) =
                            data.and_then(|d| d.get("text")).and_then(|v| v.as_str())
                        {
                            text.push_str(segment);
                            summary_text.push_str(segment);
                        }
                    }
                    "reply" => {
                        if let Some(id) = data.and_then(|d| d.get("id")).and_then(value_to_string) {
                            text.push_str(&format!("[回复:{}]", id));
                            summary_text.push_str(&format!("[回复:{}]", id));
                        } else {
                            text.push_str("[回复]");
                            summary_text.push_str("[回复]");
                        }
                    }
                    "face" => {
                        if let Some(id) = data
                            .and_then(|d| d.get("id"))
                            .and_then(value_to_string)
                        {
                            let placeholder = face_inline_placeholder(&id)
                                .unwrap_or_else(|| format!("[face:{}]", id));
                            text.push_str(&placeholder);
                            summary_text.push_str(&placeholder);
                        }
                    }
                    "json" => {
                        text.push_str("[卡片]");
                        summary_text.push_str("[卡片]");
                    }
                    "forward" => {
                        if let Some(id) =
                            data.and_then(|d| d.get("id")).and_then(value_to_string)
                        {
                            text.push_str(&format!("[合并转发:{}]", id));
                            summary_text.push_str(&format!("[合并转发:{}]", id));
                        } else {
                            text.push_str("[合并转发]");
                            summary_text.push_str("[合并转发]");
                        }
                    }
                    "poke" => {
                        text.push_str("[戳一戳]");
                        summary_text.push_str("[戳一戳]");
                    }
                    "image" => {
                        let kind = image_kind_from_data(data);
                        if let Some(reference) = extract_reference(data) {
                            attachments.push(IngressAttachment {
                                kind,
                                name: data
                                    .and_then(|d| d.get("name"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                reference,
                                size_bytes: extract_attachment_size(data),
                            });
                        } else {
                            summary_text.push_str(attachment_placeholder(kind));
                        }
                    }
                    "video" | "file" | "record" => {
                        if segment_type == "record" {
                            text.push_str("[语音]");
                            summary_text.push_str("[语音]");
                        }
                        if let Some(reference) = extract_reference(data) {
                            attachments.push(IngressAttachment {
                                kind: match segment_type {
                                    "video" => MediaKind::Video,
                                    "file" => MediaKind::File,
                                    "record" => MediaKind::Audio,
                                    _ => MediaKind::Other,
                                },
                                name: data
                                    .and_then(|d| d.get("name"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string()),
                                reference,
                                size_bytes: extract_attachment_size(data),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    ExtractedMessage {
        text: text.trim().to_string(),
        summary_text: summary_text.trim().to_string(),
        attachments,
    }
}

fn image_kind_from_data(data: Option<&Value>) -> MediaKind {
    match image_sub_type(data) {
        Some(0) => MediaKind::Image,
        Some(_) => MediaKind::Sticker,
        None => MediaKind::Sticker,
    }
}

fn image_sub_type(data: Option<&Value>) -> Option<i64> {
    let data = data?;
    value_opt_to_i64(
        data.get("sub_type")
            .or_else(|| data.get("subType"))
            .or_else(|| data.get("subtype")),
    )
}

fn extract_reference(data: Option<&Value>) -> Option<MediaReference> {
    let data = data?;
    if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
        return Some(MediaReference::RemoteUrl { url: url.to_string() });
    }
    if let Some(file) = data.get("file").and_then(|v| v.as_str()) {
        return Some(MediaReference::RemoteUrl { url: file.to_string() });
    }
    if let Some(path) = data.get("path").and_then(|v| v.as_str()) {
        return Some(MediaReference::RemoteUrl { url: path.to_string() });
    }
    None
}

fn extract_attachment_size(data: Option<&Value>) -> Option<u64> {
    let data = data?;
    let size = value_opt_to_i64(
        data.get("size")
            .or_else(|| data.get("file_size"))
            .or_else(|| data.get("filesize")),
    )?;
    u64::try_from(size).ok().filter(|value| *value > 0)
}

fn extract_cq_faces(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    let mut remaining = message;
    loop {
        let Some(start) = remaining.find("[CQ:face") else {
            output.push_str(remaining);
            break;
        };
        let (prefix, rest) = remaining.split_at(start);
        output.push_str(prefix);

        let Some(end) = rest.find(']') else {
            output.push_str(rest);
            break;
        };
        let segment = &rest[..=end];
        if let Some(face_id) = parse_cq_face_id(segment) {
            if let Some(placeholder) = face_inline_placeholder(&face_id) {
                output.push_str(&placeholder);
            } else {
                output.push_str(&format!("[face:{}]", face_id));
            }
            remaining = &rest[end + 1..];
            continue;
        }

        output.push_str(segment);
        remaining = &rest[end + 1..];
    }
    output
}

fn parse_cq_face_id(segment: &str) -> Option<String> {
    let trimmed = segment
        .strip_prefix('[')
        .unwrap_or(segment)
        .strip_suffix(']')
        .unwrap_or(segment);
    let params = trimmed.strip_prefix("CQ:face")?;
    let params = params.strip_prefix(',').unwrap_or(params);
    for part in params.split(',') {
        if let Some(value) = part.strip_prefix("id=") {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn face_inline_placeholder(face_id: &str) -> Option<String> {
    let face_id = normalize_face_id(face_id)?;
    let path = Path::new("res").join("face").join(format!("{}.png", face_id));
    if !path.exists() {
        return None;
    }
    Some(format!("[[face:{}]]", face_id))
}

fn normalize_face_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_string())
}

async fn fetch_review_code_from_reply(
    runtime: &NapCatRuntimeConfig,
    reply_id: &str,
) -> Option<ReviewCode> {
    let api_base = napcat_http_base(&runtime.napcat.ws_url)?;
    let client = Client::builder().timeout(Duration::from_secs(6)).build().ok()?;
    let url = format!("{}/get_msg", api_base);
    let mut req = client.post(url).json(&json!({ "message_id": reply_id }));
    if let Some(token) = runtime.napcat.access_token.as_ref() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req.send().await.ok()?;
    let status = resp.status();
    let body: Value = resp.json().await.ok()?;
    if !status.is_success() {
        debug_log!(
            "napcat get_msg failed: message_id={} status={} body={}",
            reply_id,
            status,
            body
        );
        return None;
    }
    if body.get("status").and_then(|v| v.as_str()) != Some("ok") {
        debug_log!(
            "napcat get_msg status not ok: message_id={} status={:?} body={}",
            reply_id,
            body.get("status"),
            body
        );
        return None;
    }
    let data = body.get("data")?;
    let message_value = data.get("message").or_else(|| data.get("raw_message"));
    let extracted = extract_message_lite(message_value);
    let review_code = parse_review_code(&extracted.text);
    if review_code.is_none() {
        debug_log!(
            "napcat get_msg missing review code: message_id={} text_len={}",
            reply_id,
            extracted.text.len()
        );
    }
    review_code
}

async fn parse_review_command(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    user_id: &str,
    group_id: &str,
    review_code: Option<ReviewCode>,
    action: ReviewAction,
    reply_id: Option<String>,
    now_ms: i64,
) -> Option<Command> {
    let mut review_code = review_code;
    let mut review_id = None;
    let mut audit_msg_id = reply_id.clone();
    let mut is_processed = false;
    let mut reply_missing = false;

    {
        let guard = state.lock().await;
        if let Some(reply_id) = reply_id.as_ref() {
            if let Some(mapped) = guard.audit_msg_to_review.get(reply_id.as_str()) {
                review_id = Some(*mapped);
                review_code = None;
            } else {
                reply_missing = true;
            }
        }

        if review_id.is_none() {
            if let Some(code) = review_code {
                if let Some(mapped) = guard.review_by_code.get(&code).copied() {
                    review_id = Some(mapped);
                    review_code = None;
                    audit_msg_id = None;
                }
            }
        }

    }

    if review_id.is_none() && review_code.is_none() && reply_missing {
        if let Some(reply_id) = reply_id.as_ref() {
            if let Some(code) = fetch_review_code_from_reply(runtime, reply_id).await {
                review_code = Some(code);
                audit_msg_id = None;
                reply_missing = false;
            }
        }
    }

    if review_id.is_none() {
        if let Some(code) = review_code {
            let mapped = {
                let guard = state.lock().await;
                guard.review_by_code.get(&code).copied()
            };
            if let Some(mapped) = mapped {
                review_id = Some(mapped);
                review_code = None;
                audit_msg_id = None;
            }
        }
    }

    if reply_missing && review_id.is_none() && review_code.is_none() {
        send_group_text(out_tx, group_id, "找不到回复的消息").await;
        return None;
    }

    if review_id.is_none() && audit_msg_id.is_none() && review_code.is_none() {
        send_group_text(out_tx, group_id, "请回复审核消息或提供编号").await;
        return None;
    }

    if let Some(resolved_id) = review_id {
        let guard = state.lock().await;
        is_processed = guard.processed_reviews.contains(&resolved_id);
    }

    if is_processed {
        send_group_text(out_tx, group_id, "此稿件已被处理").await;
        return None;
    }

    send_group_text(out_tx, group_id, "已收到指令").await;

    Some(Command::ReviewAction(ReviewActionCommand {
        review_id,
        review_code,
        audit_msg_id,
        action,
        operator_id: user_id.to_string(),
        now_ms,
        tz_offset_minutes: runtime.tz_offset_minutes,
    }))
}

fn parse_audit_command(text: &str, has_reply: bool) -> Option<AuditCommand> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_help_command(trimmed) {
        return Some(AuditCommand::Global(GlobalAction::Help));
    }

    let (first, rest) = split_first_token_with_rest(trimmed)?;

    if is_digits_unbounded(first) {
        let review_code = first.parse::<ReviewCode>().ok()?;
        let (command, args_text) = split_first_token_with_rest(rest)?;
        let args_text = args_text.trim_start();
        let action = parse_review_action(command, args_text, true)?;
        return Some(AuditCommand::Review {
            review_code: Some(review_code),
            action,
        });
    }

    if let Some(action) = parse_review_action(first, &rest, false) {
        return Some(AuditCommand::Review {
            review_code: None,
            action,
        });
    }

    if let Some(action) = parse_global_action(first, &rest) {
        return Some(AuditCommand::Global(action));
    }

    if has_reply {
        if let Some(action) = parse_review_action(first, &rest, true) {
            return Some(AuditCommand::Review {
                review_code: None,
                action,
            });
        }
    }

    None
}

fn split_first_token_with_rest(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let mut iter = input.splitn(2, char::is_whitespace);
    let first = iter.next().unwrap_or("");
    let rest = iter.next().unwrap_or("");
    if first.is_empty() {
        None
    } else {
        Some((first, rest))
    }
}

fn parse_review_action(command: &str, rest: &str, allow_quick_reply: bool) -> Option<ReviewAction> {
    let rest = rest.trim();
    let action = match command {
        "是" => ReviewAction::Approve,
        "否" => ReviewAction::Skip,
        "等" => ReviewAction::Defer { delay_ms: 180_000 },
        "删" => ReviewAction::Delete,
        "拒" => ReviewAction::Reject,
        "立即" => ReviewAction::Immediate,
        "刷新" => ReviewAction::Refresh,
        "重渲染" => ReviewAction::Rerender,
        "消息全选" => ReviewAction::SelectAllMessages,
        "匿" => ReviewAction::ToggleAnonymous,
        "扩列审查" => ReviewAction::ExpandAudit,
        "展示" => ReviewAction::Show,
        "评论" => ReviewAction::Comment {
            text: rest.to_string(),
        },
        "回复" => ReviewAction::Reply {
            text: rest.to_string(),
        },
        "合并" => {
            let target = rest.split_whitespace().next()?;
            let review_code = target.parse::<ReviewCode>().ok()?;
            ReviewAction::Merge { review_code }
        }
        "拉黑" => ReviewAction::Blacklist {
            reason: if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            },
        },
        _ => {
            if allow_quick_reply {
                ReviewAction::QuickReply {
                    key: command.to_string(),
                }
            } else {
                return None;
            }
        }
    };

    Some(action)
}

fn parse_global_action(command: &str, rest: &str) -> Option<GlobalAction> {
    let rest = rest.trim();
    match command {
        "帮助" => Some(GlobalAction::Help),
        "调出" => parse_review_code(rest).map(|review_code| GlobalAction::Recall { review_code }),
        "信息" => parse_review_code(rest).map(|review_code| GlobalAction::Info { review_code }),
        "手动重新登录" => Some(GlobalAction::ManualRelogin),
        "自动重新登录" => Some(GlobalAction::AutoRelogin),
        "待处理" => Some(GlobalAction::PendingList),
        "删除待处理" => Some(GlobalAction::PendingClear),
        "删除暂存区" => Some(GlobalAction::SendQueueClear),
        "发送暂存区" => Some(GlobalAction::SendQueueFlush),
        "清理发送中" => Some(GlobalAction::SendInFlightClear),
        "列出拉黑" => Some(GlobalAction::BlacklistList),
        "取消拉黑" => parse_first_token(rest)
            .map(|sender_id| GlobalAction::BlacklistRemove { sender_id }),
        "设定编号" => parse_u64(rest).map(|value| GlobalAction::SetExternalNumber { value }),
        "快捷回复" => parse_quick_reply_action(rest),
        "自检" => Some(GlobalAction::SelfCheck),
        "系统修复" => Some(GlobalAction::SystemRepair),
        _ => None,
    }
}

fn parse_quick_reply_action(rest: &str) -> Option<GlobalAction> {
    let mut tokens = rest.split_whitespace();
    let sub = tokens.next();
    match sub {
        None => Some(GlobalAction::QuickReplyList),
        Some("添加") => {
            let payload = tokens.collect::<Vec<_>>().join(" ");
            let (key, text) = payload.split_once('=')?;
            let key = key.trim();
            let text = text.trim();
            if key.is_empty() || text.is_empty() {
                return None;
            }
            Some(GlobalAction::QuickReplyAdd {
                key: key.to_string(),
                text: text.to_string(),
            })
        }
        Some("删除") => {
            let key = tokens.next()?.trim();
            if key.is_empty() {
                return None;
            }
            Some(GlobalAction::QuickReplyDelete {
                key: key.to_string(),
            })
        }
        _ => None,
    }
}

fn parse_review_code(text: &str) -> Option<ReviewCode> {
    let token = text.split_whitespace().next()?;
    let trimmed = token.strip_prefix('#').unwrap_or(token);
    trimmed.parse::<ReviewCode>().ok()
}

fn parse_first_token(text: &str) -> Option<String> {
    text.split_whitespace().next().map(|token| token.to_string())
}

fn parse_u64(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse::<u64>().ok()
}

fn build_pending_list_text(state: &NapCatState, group_id: &str) -> String {
    let mut pending_reviews = state
        .review_info
        .iter()
        .filter_map(|(review_id, info)| {
            if info.group_id != group_id {
                return None;
            }
            if state.processed_reviews.contains(review_id) {
                return None;
            }
            Some(info.review_code)
        })
        .collect::<Vec<_>>();
    pending_reviews.sort_unstable();
    let pending_review_labels = pending_reviews
        .iter()
        .map(|code| format!("#{}", code))
        .collect::<Vec<_>>();

    let mut pending_send = state
        .send_plans
        .iter()
        .filter_map(|(post_id, plan)| {
            if plan.group_id != group_id {
                return None;
            }
            Some((
                plan.not_before_ms,
                plan.priority,
                plan.seq,
                post_label(state, *post_id),
            ))
        })
        .collect::<Vec<_>>();
    pending_send.sort_by(|a, b| (a.0, a.1, a.2, &a.3).cmp(&(b.0, b.1, b.2, &b.3)));
    let pending_send_labels = pending_send
        .into_iter()
        .map(|(_, _, _, label)| label)
        .collect::<Vec<_>>();

    let mut sending = state
        .sending
        .iter()
        .filter_map(|(post_id, info)| {
            if info.group_id != group_id {
                return None;
            }
            Some((info.started_at_ms, post_label(state, *post_id)))
        })
        .collect::<Vec<_>>();
    sending.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let sending_labels = sending
        .into_iter()
        .map(|(_, label)| label)
        .collect::<Vec<_>>();

    if pending_review_labels.is_empty() && pending_send_labels.is_empty() && sending_labels.is_empty()
    {
        return "待处理为空".to_string();
    }

    let mut lines = Vec::new();
    lines.push("待处理列表:".to_string());
    lines.push(format!(
        "待审核({}): {}",
        pending_review_labels.len(),
        format_list(&pending_review_labels),
    ));
    lines.push(format!(
        "待发送({}): {}",
        pending_send_labels.len(),
        format_list(&pending_send_labels),
    ));
    lines.push(format!(
        "发送中({}): {}",
        sending_labels.len(),
        format_list(&sending_labels),
    ));
    lines.join("\n")
}

fn build_blacklist_list_text(state: &NapCatState, group_id: &str) -> String {
    let Some(entries) = state.blacklist.get(group_id) else {
        return "黑名单为空".to_string();
    };
    if entries.is_empty() {
        return "黑名单为空".to_string();
    }
    let mut lines = entries
        .iter()
        .map(|(sender_id, reason)| {
            let reason = reason.as_deref().unwrap_or("无");
            format!("{} -> {}", sender_id, reason)
        })
        .collect::<Vec<_>>();
    lines.sort();
    let count = lines.len();
    lines.insert(0, format!("黑名单({}):", count));
    lines.join("\n")
}

fn post_label(state: &NapCatState, post_id: PostId) -> String {
    let review_code = state.post_review_code.get(&post_id).copied();
    let external_code = state.post_external_code.get(&post_id).copied();
    match (external_code, review_code) {
        (Some(external), Some(review)) => format!("#{}/{}", external, review),
        (Some(external), None) => format!("#{}", external),
        (None, Some(review)) => format!("#{}", review),
        (None, None) => format!("post:{}", id128_hex(post_id.0)),
    }
}

fn post_label_plain(state: &NapCatState, post_id: PostId) -> String {
    post_label(state, post_id)
        .trim_start_matches('#')
        .to_string()
}

fn post_code_text(state: &NapCatState, post_id: PostId) -> Option<String> {
    state
        .post_external_code
        .get(&post_id)
        .map(|code| code.to_string())
        .or_else(|| state.post_review_code.get(&post_id).map(|code| code.to_string()))
}

fn resolve_post_submitter(state: &NapCatState, post_id: PostId) -> Option<String> {
    let ingress_ids = state.post_ingress.get(&post_id)?;
    resolve_post_submitter_with_ingress(state, ingress_ids)
}

fn resolve_post_submitter_with_ingress(
    state: &NapCatState,
    ingress_ids: &[IngressId],
) -> Option<String> {
    ingress_ids.iter().find_map(|ingress_id| {
        let summary = state.ingress_summary.get(ingress_id)?;
        let trimmed = summary.user_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_review_submitter(state: &NapCatState, review_id: ReviewId) -> Option<(String, String)> {
    let info = state.review_info.get(&review_id)?;
    let user_id = state
        .review_submitter
        .get(&review_id)
        .cloned()
        .or_else(|| resolve_post_submitter(state, info.post_id))?;
    Some((info.group_id.clone(), user_id))
}

fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        "无".to_string()
    } else {
        items.join(" ")
    }
}

fn extract_sender_name(value: &Value) -> Option<String> {
    let sender = value.get("sender")?;
    let card = sender.get("card").and_then(|v| v.as_str()).map(|s| s.trim());
    if let Some(card) = card {
        if !card.is_empty() {
            return Some(card.to_string());
        }
    }
    let nickname = sender
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.trim());
    nickname.filter(|name| !name.is_empty()).map(|name| name.to_string())
}

const SUMMARY_LINE_MAX_CHARS: usize = 120;

fn build_audit_message(
    review_code: ReviewCode,
    post_id: PostId,
    ingress_ids: &[IngressId],
    ingress_map: &HashMap<IngressId, IngressSummary>,
    preview_image: Option<String>,
    blob_paths: &HashMap<BlobId, String>,
    is_safe: bool,
) -> AuditMessage {
    let mut images = Vec::new();
    if let Some(preview) = preview_image {
        images.push(preview);
    }
    if ingress_ids.is_empty() {
        return AuditMessage {
            text: format!("#{} post {}", review_code, post_id.0),
            images,
        };
    }

    let mut lines = Vec::new();
    let mut user_id = None;
    let mut sender_name = None;

    for ingress_id in ingress_ids {
        if let Some(summary) = ingress_map.get(ingress_id) {
            if user_id.is_none() {
                user_id = Some(summary.user_id.clone());
                sender_name = summary
                    .sender_name
                    .clone()
                    .filter(|name| !name.trim().is_empty());
            }

            if let Some(line) = sanitize_summary_line(&summary.text) {
                lines.push(line);
            }
            for attachment in &summary.attachments {
                if attachment.kind != MediaKind::Image {
                    lines.push(attachment_placeholder(attachment.kind).to_string());
                }
                if let Some(image) = image_source_from_attachment(attachment, blob_paths) {
                    images.push(image);
                }
            }
        }
    }

    let safety_text = if is_safe { "安全" } else { "不安全" };
    let header = match user_id {
        Some(user_id) => {
            let display_name = sender_name.unwrap_or_else(|| user_id.clone());
            format!(
                "#{} 来自 {}({}) 系统判断{}",
                review_code, display_name, user_id, safety_text
            )
        }
        None => format!("#{} post {} 系统判断{}", review_code, post_id.0, safety_text),
    };

    let mut text = String::new();
    text.push_str(&header);
    text.push('\n');
    text.push_str("消息概览：");
    if lines.is_empty() {
        text.push('\n');
        text.push_str(" （空）");
    } else {
        for line in lines {
            text.push('\n');
            text.push(' ');
            text.push_str(&line);
        }
    }
    if !images.is_empty() {
        text.push('\n');
        text.push_str("图片：");
    }

    AuditMessage { text, images }
}

fn sanitize_summary_line(text: &str) -> Option<String> {
    let with_cq = replace_face_placeholders_with_cq(text);
    let flattened = with_cq.replace('\n', " ");
    let normalized = flattened.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= SUMMARY_LINE_MAX_CHARS {
            break;
        }
        out.push(ch);
    }
    if trimmed.chars().count() > SUMMARY_LINE_MAX_CHARS {
        out.push_str("...");
    }
    Some(out)
}

fn replace_face_placeholders_with_cq(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'[' && bytes.get(idx + 1) == Some(&b'[') {
            let rest = &text[idx..];
            if rest.starts_with("[[face:") {
                let after_prefix = idx + "[[face:".len();
                if after_prefix <= text.len() {
                    if let Some(close) = text[after_prefix..].find("]]") {
                        let face_id = &text[after_prefix..after_prefix + close];
                        if !face_id.is_empty() && face_id.chars().all(|c| c.is_ascii_digit()) {
                            out.push_str("[CQ:face,id=");
                            out.push_str(face_id);
                            out.push(']');
                            idx = after_prefix + close + 2;
                            continue;
                        }
                    }
                }
            }
        }
        let ch = text[idx..].chars().next().unwrap();
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

fn message_segments_from_text(text: &str) -> Vec<Value> {
    let mut segments = Vec::new();
    let mut buffer = String::new();
    let bytes = text.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'[' {
            let rest = &text[idx..];
            if let Some((face_id, consumed)) = parse_face_marker(rest) {
                flush_text_segment(&mut segments, &mut buffer);
                segments.push(serde_json::json!({
                    "type": "face",
                    "data": { "id": face_id }
                }));
                idx += consumed;
                continue;
            }
        }
        let ch = text[idx..].chars().next().unwrap();
        buffer.push(ch);
        idx += ch.len_utf8();
    }
    flush_text_segment(&mut segments, &mut buffer);
    segments
}

fn flush_text_segment(segments: &mut Vec<Value>, buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }
    segments.push(serde_json::json!({
        "type": "text",
        "data": { "text": buffer.clone() }
    }));
    buffer.clear();
}

fn parse_face_marker(rest: &str) -> Option<(String, usize)> {
    if let Some(found) = parse_face_placeholder(rest, "[[face:", "]]") {
        return Some(found);
    }
    if let Some(found) = parse_face_placeholder(rest, "[face:", "]") {
        return Some(found);
    }
    if rest.starts_with("[CQ:face") {
        let end = rest.find(']')?;
        let segment = &rest[..=end];
        let face_id = parse_cq_face_id(segment)?;
        let face_id = normalize_face_id(&face_id)?;
        return Some((face_id, end + 1));
    }
    None
}

fn parse_face_placeholder(rest: &str, prefix: &str, suffix: &str) -> Option<(String, usize)> {
    if !rest.starts_with(prefix) {
        return None;
    }
    let after_prefix = prefix.len();
    let close = rest[after_prefix..].find(suffix)?;
    let face_id = &rest[after_prefix..after_prefix + close];
    let face_id = normalize_face_id(face_id)?;
    Some((face_id, after_prefix + close + suffix.len()))
}

fn attachment_placeholder(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "[图片]",
        MediaKind::Video => "[视频]",
        MediaKind::File => "[文件]",
        MediaKind::Audio => "[音频]",
        MediaKind::Other => "[附件]",
        MediaKind::Sticker => "[表情]",
    }
}

fn image_source_from_attachment(
    attachment: &IngressAttachment,
    blob_paths: &HashMap<BlobId, String>,
) -> Option<String> {
    if attachment.kind != MediaKind::Image {
        return None;
    }
    match &attachment.reference {
        MediaReference::Blob { blob_id } => {
            if let Some(bytes) = blob_cache::get_bytes(*blob_id) {
                return Some(format!("base64://{}", STANDARD.encode(bytes.as_ref())));
            }
            blob_paths
                .get(blob_id)
                .map(|path| file_uri_from_path(Path::new(path)))
        }
        MediaReference::RemoteUrl { url } => {
            if url.starts_with("file://") || url.starts_with("data:") || url.starts_with("base64://")
            {
                return Some(url.clone());
            }
            if Path::new(url).exists() {
                return Some(file_uri_from_path(Path::new(url)));
            }
            None
        }
    }
}

fn rendered_png_preview(post_id: PostId) -> Option<String> {
    let blob_id = rendered_png_blob_id(post_id);
    if let Some(bytes) = blob_cache::get_bytes(blob_id) {
        return Some(format!("base64://{}", STANDARD.encode(bytes.as_ref())));
    }
    let path = rendered_png_path(post_id);
    let meta = fs::metadata(&path).ok()?;
    if meta.len() == 0 {
        return None;
    }
    Some(file_uri_from_path(&path))
}

fn rendered_png_blob_id(post_id: PostId) -> BlobId {
    derive_blob_id(&[&post_id.to_be_bytes(), b"png"])
}

fn rendered_png_path(post_id: PostId) -> PathBuf {
    let blob_id = rendered_png_blob_id(post_id);
    let filename = format!("{}.png", id128_hex(blob_id.0));
    blob_root().join("png").join(filename)
}

fn blob_root() -> PathBuf {
    std::env::var("OQQWALL_BLOB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/blobs"))
}

fn file_uri_from_path(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    format!("file://{}", absolute.to_string_lossy())
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

const HELP_TEXT: &str = r#"全局指令:
这些是可以在任何时刻@本账号调用的指令
语法: @本账号/次要账号 指令

帮助:
查看这个帮助列表

调出:
调出曾经接收到过的投稿
用法：调出 <review_code>

信息:
查询该编号的接收者、发送者、所属组、处理后信息
用法：信息 <review_code>

手动重新登录:
扫码登陆QQ空间

自动重新登录:
尝试自动登录QQ空间

待处理:
列出当前等待处理投稿（按账号组过滤）

删除待处理:
清空待处理列表，相当于对列表中的所有项目执行"删"审核指令

删除暂存区:
清空暂存区内容，并回滚外部编号

发送暂存区:
将暂存区内容发送到QQ空间

清理发送中:
清理卡住的发送中状态，并重新入队

列出拉黑:
列出当前被拉黑账号列表

取消拉黑:
取消对某账号拉黑
用法：取消拉黑 <senderid>

设定编号:
设定下一条说说外部编号（纯数字）
用法：设定编号 <纯数字>

快捷回复:
查看当前账号组配置的快捷回复列表

快捷回复 添加:
添加快捷回复指令
用法：快捷回复 添加 指令名=内容

快捷回复 删除:
删除指定快捷回复指令
用法：快捷回复 删除 指令名

自检:
系统与服务自检

系统修复:
重启服务并重建连接（谨慎使用）


审核指令:
这些指令仅在稿件审核流程中要求您发送指令时可用
语法: @本账号 review_code 指令
或 回复审核消息 指令

是:
发送，并给稿件发送者发送成功提示

否:
机器跳过此条，人工处理（常用于分段/匿名失败或含视频）

匿:
切换匿名状态，处理后会再次询问指令

等:
等待180秒，然后重新执行分段-渲染-审核流程

删:
此条不发送，也不用人工发送

拒:
拒绝稿件，并给发送者发送被拒提示

立即:
立刻发送暂存区全部投稿，并立即把当前投稿单发

刷新:
重新进行“聊天记录->图片”的过程

重渲染:
重新进行渲染，不重做分段

消息全选:
强制把本次投稿所有消息作为内容并重渲染

扩列审查:
扩列审核流程（抓等级/空间/名片/二维码等）

评论:
增加文本评论，处理后再次询问
用法：评论 <文本>

回复:
向投稿人发送一条信息
用法：回复 <文本>

展示:
展示稿件内容

拉黑:
不再接收来自此人的投稿
用法：拉黑 [理由]

快捷回复指令:
使用预设模板向投稿人发送消息
用法：<快捷指令名>"#;

fn is_admin_sender(value: &Value) -> bool {
    value
        .get("sender")
        .and_then(|sender| sender.get("role"))
        .and_then(|role| role.as_str())
        .map(|role| role == "admin" || role == "owner")
        .unwrap_or(false)
}

fn is_help_command(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == "帮助" || trimmed.eq_ignore_ascii_case("help")
}

async fn send_group_text(out_tx: &mpsc::Sender<String>, group_id: &str, text: &str) {
    let payload = serde_json::json!({
        "action": "send_group_msg",
        "params": {
            "group_id": json_id(group_id),
            "message": [{"type": "text", "data": {"text": text}}]
        }
    });
    let _ = out_tx.send(payload.to_string()).await;
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn value_opt_to_string(value: Option<&Value>) -> Option<String> {
    value.and_then(value_to_string)
}

fn value_opt_to_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn value_opt_to_u8(value: Option<&Value>) -> Option<u8> {
    match value? {
        Value::Number(n) => n.as_u64().and_then(|v| u8::try_from(v).ok()),
        Value::String(s) => s.parse::<u8>().ok(),
        _ => None,
    }
}

fn inbound_timestamp_ms(value: &Value) -> i64 {
    value
        .get("time")
        .and_then(|v| v.as_i64())
        .map(|sec| sec.saturating_mul(1000))
        .unwrap_or_else(now_ms)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn next_echo(state: &mut NapCatState) -> String {
    state.next_echo = state.next_echo.saturating_add(1);
    format!("echo-{}", state.next_echo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_help_and_review_with_code() {
        assert_eq!(
            parse_audit_command("帮助", false),
            Some(AuditCommand::Global(GlobalAction::Help))
        );
        assert_eq!(
            parse_audit_command("help", false),
            Some(AuditCommand::Global(GlobalAction::Help))
        );
        assert_eq!(
            parse_audit_command("123 是", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ReviewAction::Approve,
            })
        );
        assert_eq!(
            parse_audit_command("123 删", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ReviewAction::Delete,
            })
        );
        assert_eq!(
            parse_audit_command("123 拒", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ReviewAction::Reject,
            })
        );
        assert_eq!(
            parse_audit_command("123 合并 456", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ReviewAction::Merge { review_code: 456 },
            })
        );
    }

    #[test]
    fn parse_global_and_quick_reply_actions() {
        assert_eq!(
            parse_audit_command("调出 42", false),
            Some(AuditCommand::Global(GlobalAction::Recall { review_code: 42 }))
        );
        assert_eq!(
            parse_audit_command("调出 #42", false),
            Some(AuditCommand::Global(GlobalAction::Recall { review_code: 42 }))
        );
        assert_eq!(
            parse_audit_command("清理发送中", false),
            Some(AuditCommand::Global(GlobalAction::SendInFlightClear))
        );
        assert_eq!(
            parse_audit_command("快捷回复 添加 hi=hello", false),
            Some(AuditCommand::Global(GlobalAction::QuickReplyAdd {
                key: "hi".to_string(),
                text: "hello".to_string(),
            }))
        );
    }

    #[test]
    fn parse_quick_reply_requires_reply_context() {
        assert_eq!(parse_audit_command("谢谢", false), None);
        assert_eq!(
            parse_audit_command("谢谢", true),
            Some(AuditCommand::Review {
                review_code: None,
                action: ReviewAction::QuickReply {
                    key: "谢谢".to_string(),
                },
            })
        );
    }

    #[test]
    fn parse_reply_text_preserves_spaces() {
        assert_eq!(
            parse_audit_command("123 回复 hello world", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ReviewAction::Reply {
                    text: "hello world".to_string(),
                },
            })
        );
        assert_eq!(
            parse_audit_command("123 回复  hello   world", false),
            Some(AuditCommand::Review {
                review_code: Some(123),
                action: ReviewAction::Reply {
                    text: "hello   world".to_string(),
                },
            })
        );
        assert_eq!(
            parse_audit_command("回复  你好  世界", true),
            Some(AuditCommand::Review {
                review_code: None,
                action: ReviewAction::Reply {
                    text: "你好  世界".to_string(),
                },
            })
        );
    }

    #[test]
    fn message_segments_from_text_parses_faces() {
        let segments = message_segments_from_text("a[[face:12]]b[face:34]c[CQ:face,id=56]!");
        assert_eq!(
            segments,
            vec![
                serde_json::json!({"type": "text", "data": {"text": "a"}}),
                serde_json::json!({"type": "face", "data": {"id": "12"}}),
                serde_json::json!({"type": "text", "data": {"text": "b"}}),
                serde_json::json!({"type": "face", "data": {"id": "34"}}),
                serde_json::json!({"type": "text", "data": {"text": "c"}}),
                serde_json::json!({"type": "face", "data": {"id": "56"}}),
                serde_json::json!({"type": "text", "data": {"text": "!"}}),
            ]
        );
    }
}
