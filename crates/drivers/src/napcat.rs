use std::collections::HashMap;
use std::fs;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::command::{
    GlobalAction, GlobalActionCommand, ReviewAction, ReviewActionCommand,
};
use oqqwall_rust_core::draft::{IngressAttachment, IngressMessage, MediaKind, MediaReference};
use oqqwall_rust_core::event::{DraftEvent, Event, IngressEvent, ReviewEvent};
use oqqwall_rust_core::ids::{IngressId, PostId, ReviewCode, ReviewId};
use oqqwall_rust_core::{derive_blob_id, Command, IngressCommand};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        eprintln!($($arg)*);
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
}

#[derive(Debug, Clone)]
struct ReviewInfo {
    review_code: ReviewCode,
    post_id: PostId,
}

#[derive(Debug, Clone)]
struct IngressSummary {
    user_id: String,
    sender_name: Option<String>,
    text: String,
    attachments: Vec<IngressAttachment>,
}

#[derive(Debug, Clone)]
struct AuditMessage {
    text: String,
    images: Vec<String>,
}

#[derive(Debug, Clone)]
enum PendingAction {
    SendAuditMessage { review_id: ReviewId },
}

#[derive(Debug, Clone)]
enum AuditCommand {
    Review {
        review_code: Option<ReviewCode>,
        action: ReviewAction,
    },
    Global(GlobalAction),
}

#[derive(Default)]
struct NapCatState {
    review_info: HashMap<ReviewId, ReviewInfo>,
    review_by_code: HashMap<ReviewCode, ReviewId>,
    ingress_summary: HashMap<IngressId, IngressSummary>,
    post_ingress: HashMap<PostId, Vec<IngressId>>,
    audit_msg_to_review: HashMap<String, ReviewId>,
    pending: HashMap<String, PendingAction>,
    next_echo: u64,
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
        let state = Arc::new(Mutex::new(NapCatState::default()));
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
                        Err(err) => {
                            debug_log!("napcat ws read error: {}", err);
                            break;
                        }
                    };
                    if !msg.is_text() {
                        debug_log!("napcat ws ignoring non-text message");
                        continue;
                    }
                    let text = match msg.to_text() {
                        Ok(text) => text,
                        Err(err) => {
                            debug_log!("napcat ws text decode error: {}", err);
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
            let bus_task = tokio::spawn(async move {
                loop {
                    let env = match bus_task_rx.recv().await {
                        Ok(env) => env,
                        Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    };

                    let action = build_action_from_event(&runtime_bus, &state_bus, env.event).await;
                    if let Some(action) = action {
                        debug_log!(
                            "napcat ws outbound action: group_id={} bytes={}",
                            runtime_bus.group_id,
                            action.len()
                        );
                        let _ = out_tx.send(action).await;
                    }
                }
            });

            let _ = tokio::join!(writer, reader, bus_task);
            debug_log!("napcat ws disconnected: group_id={}", runtime.group_id);
            sleep(Duration::from_secs(2)).await;
        }
    })
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

#[cfg(debug_assertions)]
fn ws_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

fn log_ws_connect_error(
    group_id: &str,
    ws_url: &str,
    err: &tokio_tungstenite::tungstenite::Error,
) {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            let status = response.status();
            let headers = response.headers();
            let body = response.body().as_ref();
            let body_len = body.map(|bytes| bytes.len()).unwrap_or(0);
            let preview = body
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
            if let Some(preview) = preview {
                debug_log!(
                    "napcat ws connect failed: group_id={} ws_url={} status={} headers={:?} body_len={} body_preview=\"{}\"",
                    group_id,
                    ws_url_for_log(ws_url),
                    status,
                    headers,
                    body_len,
                    preview
                );
            } else {
                debug_log!(
                    "napcat ws connect failed: group_id={} ws_url={} status={} headers={:?} body_len={}",
                    group_id,
                    ws_url_for_log(ws_url),
                    status,
                    headers,
                    body_len
                );
            }
        }
        _ => {
            debug_log!(
                "napcat ws connect failed: group_id={} ws_url={} err={:?}",
                group_id,
                ws_url_for_log(ws_url),
                err
            );
        }
    }
}

async fn build_action_from_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    event: Event,
) -> Option<String> {
    match event {
        Event::Ingress(IngressEvent::MessageAccepted {
            ingress_id,
            user_id,
            sender_name,
            message,
            ..
        }) => {
            let mut guard = state.lock().await;
            guard.ingress_summary.insert(
                ingress_id,
                IngressSummary {
                    user_id,
                    sender_name,
                    text: message.text,
                    attachments: message.attachments,
                },
            );
            None
        }
        Event::Draft(DraftEvent::PostDraftCreated { post_id, ingress_ids, .. }) => {
            let mut guard = state.lock().await;
            guard.post_ingress.insert(post_id, ingress_ids);
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
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                },
            );
            guard.review_by_code.insert(review_code, review_id);
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
            None
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
            let ingress_ids = guard
                .post_ingress
                .get(&info.post_id)
                .cloned()
                .unwrap_or_default();
            let preview = rendered_png_preview(info.post_id);
            let summary = build_audit_message(
                info.review_code,
                info.post_id,
                &ingress_ids,
                &guard.ingress_summary,
                preview,
            );
            let echo = next_echo(&mut guard);
            guard.pending.insert(
                echo.clone(),
                PendingAction::SendAuditMessage { review_id },
            );

            let mut message = vec![serde_json::json!({
                "type": "text",
                "data": { "text": summary.text }
            })];
            for image in summary.images {
                message.push(serde_json::json!({
                    "type": "image",
                    "data": { "file": image }
                }));
            }
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": group_id,
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
    match pending {
        PendingAction::SendAuditMessage { review_id } => {
            let message_id = value
                .get("data")
                .and_then(|data| data.get("message_id"))
                .and_then(value_to_string)
                .unwrap_or_else(|| "unknown".to_string());
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

async fn parse_inbound_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    out_tx: &mpsc::Sender<String>,
    value: &Value,
) -> Option<Command> {
    let post_type = value.get("post_type").and_then(|v| v.as_str())?;
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
    let timestamp_ms = value
        .get("time")
        .and_then(|v| v.as_i64())
        .map(|sec| sec.saturating_mul(1000))
        .unwrap_or(0);

    let (text, attachments, reply_id) = extract_message(value.get("message"));
    debug_log!(
        "napcat inbound content: text_len={} attachments={} reply_id_present={}",
        text.len(),
        attachments.len(),
        reply_id.is_some()
    );

    if message_type == "group" {
        let chat_group_id = value_opt_to_string(value.get("group_id"))?;
        if runtime.audit_group_id.as_deref() == Some(chat_group_id.as_str()) {
            if let Some(command) = parse_audit_command(&text, reply_id.is_some()) {
                if !is_admin_sender(value) {
                    send_group_text(out_tx, &chat_group_id, "无权限执行指令").await;
                    return None;
                }
                match command {
                    AuditCommand::Global(GlobalAction::Help) => {
                        send_group_text(out_tx, &chat_group_id, "已收到指令").await;
                        send_group_text(out_tx, &chat_group_id, HELP_TEXT).await;
                        return None;
                    }
                    AuditCommand::Global(action) => {
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
            return None;
        }
        return Some(Command::Ingress(IngressCommand {
            profile_id: self_id,
            chat_id: chat_group_id.clone(),
            user_id,
            sender_name: sender_name.clone(),
            group_id: runtime.group_id.clone(),
            platform_msg_id: message_id,
            message: IngressMessage { text, attachments },
            received_at_ms: timestamp_ms,
        }));
    }

    if message_type == "private" {
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

fn extract_message(value: Option<&Value>) -> (String, Vec<IngressAttachment>, Option<String>) {
    let mut text = String::new();
    let mut attachments = Vec::new();
    let mut reply_id = None;

    match value {
        Some(Value::String(s)) => {
            text.push_str(&extract_cq_faces(s, &mut attachments));
        }
        Some(Value::Array(items)) => {
            for item in items {
                let segment_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                let data = item.get("data");
                match segment_type {
                    "text" => {
                        if let Some(segment) = data.and_then(|d| d.get("text")).and_then(|v| v.as_str()) {
                            text.push_str(segment);
                        }
                    }
                    "reply" => {
                        if let Some(id) = data
                            .and_then(|d| d.get("id"))
                            .and_then(value_to_string)
                        {
                            reply_id = Some(id);
                        }
                    }
                    "face" => {
                        if let Some(id) = data
                            .and_then(|d| d.get("id"))
                            .and_then(value_to_string)
                        {
                            if !push_face_attachment(&id, &mut attachments) {
                                text.push_str(&format!("[face:{}]", id));
                            }
                        }
                    }
                    "image" | "video" | "file" | "record" => {
                        if let Some(reference) = extract_reference(data) {
                            attachments.push(IngressAttachment {
                                kind: match segment_type {
                                    "image" => MediaKind::Image,
                                    "video" => MediaKind::Video,
                                    "file" => MediaKind::File,
                                    "record" => MediaKind::Audio,
                                    _ => MediaKind::Other,
                                },
                                name: data.and_then(|d| d.get("name")).and_then(|v| v.as_str()).map(|s| s.to_string()),
                                reference,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    (text.trim().to_string(), attachments, reply_id)
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

fn extract_cq_faces(message: &str, attachments: &mut Vec<IngressAttachment>) -> String {
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
            if push_face_attachment(&face_id, attachments) {
                let after = &rest[end + 1..];
                if needs_space(&output, after) {
                    output.push(' ');
                }
                remaining = after;
                continue;
            }
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

fn needs_space(prefix: &str, suffix: &str) -> bool {
    let left_space = prefix.chars().last().map(|c| c.is_whitespace()).unwrap_or(false);
    let right_space = suffix.chars().next().map(|c| c.is_whitespace()).unwrap_or(false);
    !left_space && !right_space
}

fn push_face_attachment(face_id: &str, attachments: &mut Vec<IngressAttachment>) -> bool {
    let Some(face_id) = normalize_face_id(face_id) else {
        return false;
    };
    let path = Path::new("res").join("face").join(format!("{}.png", face_id));
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let encoded = STANDARD.encode(bytes);
    let url = format!("data:image/png;base64,{}", encoded);
    attachments.push(IngressAttachment {
        kind: MediaKind::Image,
        name: Some(format!("face-{}.png", face_id)),
        reference: MediaReference::RemoteUrl { url },
    });
    true
}

fn normalize_face_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_string())
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

    if let Some(reply_id) = reply_id {
        let guard = state.lock().await;
        if let Some(mapped) = guard.audit_msg_to_review.get(&reply_id) {
            review_id = Some(*mapped);
            review_code = None;
        }
    }

    if review_id.is_none() {
        if let Some(code) = review_code {
            let guard = state.lock().await;
            if let Some(mapped) = guard.review_by_code.get(&code).copied() {
                review_id = Some(mapped);
                review_code = None;
            } else {
                send_group_text(out_tx, group_id, "找不到稿件").await;
                return None;
            }
        }
    }

    if review_id.is_none() && review_code.is_none() {
        send_group_text(out_tx, group_id, "请回复审核消息或提供编号").await;
        return None;
    }

    send_group_text(out_tx, group_id, "已收到指令").await;

    Some(Command::ReviewAction(ReviewActionCommand {
        review_id,
        review_code,
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

    let mut tokens = trimmed.split_whitespace();
    let first = tokens.next()?;
    let rest = tokens.collect::<Vec<_>>().join(" ");

    if is_digits(first) {
        let review_code = first.parse::<ReviewCode>().ok()?;
        let mut rest_tokens = rest.split_whitespace();
        let command = rest_tokens.next()?;
        let args_text = rest_tokens.collect::<Vec<_>>().join(" ");
        let action = parse_review_action(command, &args_text, true)?;
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

fn parse_review_action(command: &str, rest: &str, allow_quick_reply: bool) -> Option<ReviewAction> {
    let rest = rest.trim();
    let action = match command {
        "是" => ReviewAction::Approve,
        "否" => ReviewAction::Skip,
        "等" => ReviewAction::Defer { delay_ms: 180_000 },
        "删" | "拒" => ReviewAction::Reject,
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
    text.split_whitespace().next()?.parse::<ReviewCode>().ok()
}

fn parse_first_token(text: &str) -> Option<String> {
    text.split_whitespace().next().map(|token| token.to_string())
}

fn parse_u64(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse::<u64>().ok()
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
                lines.push(attachment_placeholder(attachment.kind).to_string());
                if let Some(image) = image_source_from_attachment(attachment) {
                    images.push(image);
                }
            }
        }
    }

    let header = match user_id {
        Some(user_id) => {
            let display_name = sender_name.unwrap_or_else(|| user_id.clone());
            format!("#{} 来自 {}({})", review_code, display_name, user_id)
        }
        None => format!("#{} post {}", review_code, post_id.0),
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
    let flattened = text.replace('\n', " ");
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

fn attachment_placeholder(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "[图片]",
        MediaKind::Video => "[视频]",
        MediaKind::File => "[文件]",
        MediaKind::Audio => "[音频]",
        MediaKind::Other => "[附件]",
    }
}

fn image_source_from_attachment(attachment: &IngressAttachment) -> Option<String> {
    if attachment.kind != MediaKind::Image {
        return None;
    }
    match &attachment.reference {
        MediaReference::RemoteUrl { url } => Some(url.clone()),
        MediaReference::Blob { .. } => None,
    }
}

fn rendered_png_preview(post_id: PostId) -> Option<String> {
    let path = rendered_png_path(post_id);
    if !path.exists() {
        return None;
    }
    Some(file_uri_from_path(&path))
}

fn rendered_png_path(post_id: PostId) -> PathBuf {
    let blob_id = derive_blob_id(&[&post_id.to_be_bytes(), b"png"]);
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

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
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
            "group_id": group_id,
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

fn next_echo(state: &mut NapCatState) -> String {
    state.next_echo = state.next_echo.saturating_add(1);
    format!("echo-{}", state.next_echo)
}
