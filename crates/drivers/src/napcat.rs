use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::command::{ReviewAction, ReviewActionCommand};
use oqqwall_rust_core::draft::{IngressAttachment, IngressMessage, MediaKind, MediaReference};
use oqqwall_rust_core::event::{Event, ReviewEvent};
use oqqwall_rust_core::ids::{ReviewCode, ReviewId};
use oqqwall_rust_core::{Command, IngressCommand};
use serde_json::Value;
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

#[derive(Debug, Clone)]
pub struct NapCatConfig {
    pub ws_url: String,
    pub access_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NapCatRuntimeConfig {
    pub napcat: NapCatConfig,
    pub audit_group_id: Option<String>,
    pub default_group_id: String,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone)]
struct ReviewInfo {
    review_code: ReviewCode,
    post_id: oqqwall_rust_core::PostId,
}

#[derive(Debug, Clone)]
enum PendingAction {
    SendAuditMessage { review_id: ReviewId },
}

#[derive(Default)]
struct NapCatState {
    review_info: HashMap<ReviewId, ReviewInfo>,
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
        let state = Arc::new(Mutex::new(NapCatState::default()));
        let bus_rx = bus_rx;

        loop {
            let runtime = runtime.clone();
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
                Err(_) => {
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (mut ws_write, mut ws_read) = ws_stream.split();
            let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
            let state_ref = Arc::clone(&state);

            let writer = tokio::spawn(async move {
                while let Some(msg) = out_rx.recv().await {
                    if ws_write.send(tokio_tungstenite::tungstenite::Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
            });

            let cmd_tx_read = cmd_tx.clone();
            let runtime_read = runtime.clone();
            let state_read = Arc::clone(&state_ref);
            let reader = tokio::spawn(async move {
                while let Some(msg) = ws_read.next().await {
                    let Ok(msg) = msg else { break; };
                    if !msg.is_text() {
                        continue;
                    }
                    let Ok(value) = serde_json::from_str::<Value>(msg.to_text().unwrap_or("")) else {
                        continue;
                    };
                    if let Some(echo) = value.get("echo").and_then(|v| v.as_str()) {
                        if let Some(event) =
                            handle_action_response(&state_read, echo, &value).await
                        {
                            let _ = cmd_tx_read.send(Command::DriverEvent(event)).await;
                        }
                        continue;
                    }
                    if let Some(command) =
                        parse_inbound_event(&runtime_read, &state_read, &value).await
                    {
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
                        let _ = out_tx.send(action).await;
                    }
                }
            });

            let _ = tokio::join!(writer, reader, bus_task);
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

async fn build_action_from_event(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    event: Event,
) -> Option<String> {
    match event {
        Event::Review(ReviewEvent::ReviewItemCreated {
            review_id,
            post_id,
            review_code,
        }) => {
            let mut guard = state.lock().await;
            guard.review_info.insert(
                review_id,
                ReviewInfo {
                    review_code,
                    post_id,
                },
            );
            None
        }
        Event::Review(ReviewEvent::ReviewPublished {
            review_id,
            audit_msg_id,
        }) => {
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
                return None;
            };
            let echo = next_echo(&mut guard);
            guard.pending.insert(
                echo.clone(),
                PendingAction::SendAuditMessage { review_id },
            );

            let text = format!("#{} post {}", info.review_code, info.post_id.0);
            let payload = serde_json::json!({
                "action": "send_group_msg",
                "params": {
                    "group_id": group_id,
                    "message": [{"type": "text", "data": {"text": text}}]
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
    value: &Value,
) -> Option<Command> {
    let post_type = value.get("post_type").and_then(|v| v.as_str())?;
    if post_type != "message" && post_type != "message_sent" {
        return None;
    }

    let message_type = value.get("message_type").and_then(|v| v.as_str())?;
    let user_id = value_opt_to_string(value.get("user_id"))?;
    let self_id =
        value_opt_to_string(value.get("self_id")).unwrap_or_else(|| "napcat".to_string());
    let message_id =
        value_opt_to_string(value.get("message_id")).unwrap_or_else(|| "0".to_string());
    let timestamp_ms = value
        .get("time")
        .and_then(|v| v.as_i64())
        .map(|sec| sec.saturating_mul(1000))
        .unwrap_or(0);

    let (text, attachments, reply_id) = extract_message(value.get("message"));

    if message_type == "group" {
        let group_id = value_opt_to_string(value.get("group_id"))?;
        if runtime.audit_group_id.as_deref() == Some(group_id.as_str()) {
            return parse_review_command(
                runtime,
                state,
                &user_id,
                &group_id,
                &text,
                reply_id,
                timestamp_ms,
            )
            .await;
        }
        return Some(Command::Ingress(IngressCommand {
            profile_id: self_id,
            chat_id: group_id.clone(),
            user_id,
            group_id: runtime.default_group_id.clone(),
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
            group_id: runtime.default_group_id.clone(),
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
            text.push_str(s);
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

async fn parse_review_command(
    runtime: &NapCatRuntimeConfig,
    state: &Arc<Mutex<NapCatState>>,
    user_id: &str,
    _group_id: &str,
    text: &str,
    reply_id: Option<String>,
    now_ms: i64,
) -> Option<Command> {
    let parsed = parse_review_action(text)?;
    let mut review_id = None;
    let mut review_code = parsed.0;
    let action = parsed.1;

    if let Some(reply_id) = reply_id {
        let guard = state.lock().await;
        if let Some(mapped) = guard.audit_msg_to_review.get(&reply_id) {
            review_id = Some(*mapped);
            review_code = None;
        }
    }

    Some(Command::ReviewAction(ReviewActionCommand {
        review_id,
        review_code,
        action,
        operator_id: user_id.to_string(),
        now_ms,
        tz_offset_minutes: runtime.tz_offset_minutes,
    }))
}

fn parse_review_action(text: &str) -> Option<(Option<ReviewCode>, ReviewAction)> {
    let mut tokens = text.split_whitespace();
    let first = tokens.next()?;
    let (code, command) = if is_digits(first) {
        let code = first.parse::<ReviewCode>().ok();
        (code, tokens.next()?)
    } else {
        (None, first)
    };
    let rest = tokens.collect::<Vec<_>>().join(" ");

    let action = match command {
        "是" => ReviewAction::Approve,
        "否" => ReviewAction::Skip,
        "等" => ReviewAction::Defer { delay_ms: 180_000 },
        "删" | "拒" => ReviewAction::Reject,
        "立即" => ReviewAction::Immediate,
        "刷新" => ReviewAction::Refresh,
        "重渲染" => ReviewAction::Rerender,
        "消息全选" => ReviewAction::SelectAllMessages,
        "评论" => ReviewAction::Comment { text: rest },
        "回复" => ReviewAction::Reply { text: rest },
        "拉黑" => ReviewAction::Blacklist {
            reason: if rest.is_empty() { None } else { Some(rest) },
        },
        _ => return None,
    };

    Some((code, action))
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
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
