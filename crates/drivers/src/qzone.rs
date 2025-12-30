use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::draft::{Draft, DraftBlock, MediaKind, MediaReference};
use oqqwall_rust_core::event::{DraftEvent, Event, SendEvent};
use oqqwall_rust_core::ids::{PostId, TimestampMs};
use oqqwall_rust_core::Command;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

use crate::napcat::NapCatConfig;

const EMOTION_PUBLISH_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/taotao.qzone.qq.com/cgi-bin/emotion_cgi_publish_v6";
const UPLOAD_IMAGE_URL: &str = "https://up.qzone.qq.com/cgi-bin/upload/cgi_upload_image";

#[derive(Debug, Clone)]
pub struct QzoneRuntimeConfig {
    pub napcat: NapCatConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QzoneErrorKind {
    Network,
    RiskControl,
    Account,
    Unknown,
}

#[derive(Debug, Clone)]
struct QzoneError {
    kind: QzoneErrorKind,
    message: String,
}

impl QzoneError {
    fn new(kind: QzoneErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn network(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::Network, message)
    }

    fn risk(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::RiskControl, message)
    }

    fn account(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::Account, message)
    }

    fn unknown(message: impl Into<String>) -> Self {
        Self::new(QzoneErrorKind::Unknown, message)
    }

    fn with_context(self, ctx: &str) -> Self {
        Self {
            kind: self.kind,
            message: format!("{}: {}", ctx, self.message),
        }
    }
}

#[derive(Default)]
struct QzoneState {
    drafts: HashMap<PostId, Draft>,
    attempts: HashMap<PostId, u32>,
    cookie_cache: Option<CookieCache>,
}

struct CookieCache {
    cookies: HashMap<String, String>,
    fetched_at_ms: TimestampMs,
}

pub fn spawn_qzone_sender(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    runtime: QzoneRuntimeConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let state = Arc::new(Mutex::new(QzoneState::default()));
        let mut bus_rx = bus_rx;

        loop {
            let env = match bus_rx.recv().await {
                Ok(env) => env,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            match env.event {
                Event::Draft(DraftEvent::PostDraftCreated { post_id, draft, .. }) => {
                    let mut guard = state.lock().await;
                    guard.drafts.insert(post_id, draft);
                }
                Event::Send(SendEvent::SendStarted {
                    post_id,
                    account_id,
                    started_at_ms,
                    ..
                }) => {
                    let cookies = match get_cookies(&state, &runtime.napcat).await {
                        Ok(cookies) => cookies,
                        Err(err) => {
                            let retry_at = started_at_ms
                                .saturating_add(retry_delay_ms(err.kind, 1));
                            let event = SendEvent::SendFailed {
                                post_id,
                                account_id,
                                attempt: 1,
                                retry_at_ms: retry_at,
                                error: format!("[{:?}] {}", err.kind, err.message),
                            };
                            let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                            continue;
                        }
                    };
                    let client = match QzoneClient::from_cookie_map(cookies) {
                        Ok(client) => client,
                        Err(err) => {
                            let retry_at = started_at_ms
                                .saturating_add(retry_delay_ms(err.kind, 1));
                            let event = SendEvent::SendFailed {
                                post_id,
                                account_id,
                                attempt: 1,
                                retry_at_ms: retry_at,
                                error: format!("[{:?}] {}", err.kind, err.message),
                            };
                            let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                            continue;
                        }
                    };
                    let draft = {
                        let guard = state.lock().await;
                        guard.drafts.get(&post_id).cloned()
                    };
                    let Some(draft) = draft else {
                        let err = QzoneError::unknown("missing draft");
                        let retry_at = started_at_ms
                            .saturating_add(retry_delay_ms(err.kind, 1));
                        let event = SendEvent::SendFailed {
                            post_id,
                            account_id,
                            attempt: 1,
                            retry_at_ms: retry_at,
                            error: format!("[{:?}] {}", err.kind, err.message),
                        };
                        let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                        continue;
                    };

                    let content = draft_to_text(&draft);
                    let images = match collect_images(&client, &draft).await {
                        Ok(images) => images,
                        Err(err) => {
                            let retry_at = started_at_ms
                                .saturating_add(retry_delay_ms(err.kind, 1));
                            let event = SendEvent::SendFailed {
                                post_id,
                                account_id,
                                attempt: 1,
                                retry_at_ms: retry_at,
                                error: format!("[{:?}] {}", err.kind, err.message),
                            };
                            let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                            continue;
                        }
                    };
                    let attempt = {
                        let mut guard = state.lock().await;
                        let entry = guard.attempts.entry(post_id).or_insert(0);
                        *entry += 1;
                        *entry
                    };

                    match client.publish_emotion(&content, &images).await {
                        Ok(tid) => {
                            let mut guard = state.lock().await;
                            guard.attempts.remove(&post_id);
                            let event = SendEvent::SendSucceeded {
                                post_id,
                                account_id,
                                finished_at_ms: started_at_ms,
                                remote_id: Some(tid),
                            };
                            let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                        }
                        Err(err) => {
                            let retry_at = started_at_ms
                                .saturating_add(retry_delay_ms(err.kind, attempt));
                            let event = SendEvent::SendFailed {
                                post_id,
                                account_id,
                                attempt,
                                retry_at_ms: retry_at,
                                error: format!("[{:?}] {}", err.kind, err.message),
                            };
                            let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                        }
                    }
                }
                _ => {}
            }
        }
    })
}

#[derive(Clone)]
struct QzoneClient {
    cookies: HashMap<String, String>,
    gtk: String,
    uin: u64,
    client: Client,
}

impl QzoneClient {
    fn from_cookie_map(cookies: HashMap<String, String>) -> Result<Self, QzoneError> {
        let skey = cookies
            .get("p_skey")
            .or_else(|| cookies.get("skey"))
            .ok_or_else(|| QzoneError::account("missing p_skey/skey"))?
            .clone();
        let gtk = generate_gtk(&skey);
        let uin = cookies
            .get("uin")
            .and_then(|value| value.trim().trim_start_matches('o').parse::<u64>().ok())
            .unwrap_or(0);
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|err| QzoneError::network(format!("reqwest init failed: {}", err)))?;
        Ok(Self {
            cookies,
            gtk,
            uin,
            client,
        })
    }

    async fn publish_emotion(
        &self,
        content: &str,
        images: &[Vec<u8>],
    ) -> Result<String, QzoneError> {
        let cookie_header = build_cookie_header(&self.cookies);
        let mut form: HashMap<&str, String> = HashMap::new();
        form.insert("syn_tweet_verson", "1".to_string());
        form.insert("paramstr", "1".to_string());
        form.insert("who", "1".to_string());
        form.insert("con", content.to_string());
        form.insert("feedversion", "1".to_string());
        form.insert("ver", "1".to_string());
        form.insert("ugc_right", "1".to_string());
        form.insert("to_sign", "0".to_string());
        form.insert("hostuin", self.uin.to_string());
        form.insert("code_version", "1".to_string());
        form.insert("format", "json".to_string());
        form.insert(
            "qzreferrer",
            format!("https://user.qzone.qq.com/{}", self.uin),
        );

        if !images.is_empty() {
            let mut pic_bos = Vec::new();
            let mut richvals = Vec::new();
            for image in images {
                let upload = self.upload_image(image).await?;
                let (picbo, richval) = get_picbo_and_richval(&upload)?;
                pic_bos.push(picbo);
                richvals.push(richval);
            }
            form.insert("pic_bo", pic_bos.join(","));
            form.insert("richtype", "1".to_string());
            form.insert("richval", richvals.join("\t"));
        }

        let res = self
            .client
            .post(EMOTION_PUBLISH_URL)
            .query(&[("g_tk", &self.gtk), ("uin", &self.uin.to_string())])
            .header("referer", format!("https://user.qzone.qq.com/{}", self.uin))
            .header("origin", "https://user.qzone.qq.com")
            .header("cookie", cookie_header)
            .form(&form)
            .send()
            .await
            .map_err(|err| classify_reqwest_error("publish request", err))?;

        if !res.status().is_success() {
            return Err(classify_http_status("publish http status", res.status().as_u16()));
        }

        let body = res
            .text()
            .await
            .map_err(|err| classify_reqwest_error("publish read body", err))?;
        let json: Value = serde_json::from_str(&body)
            .map_err(|err| QzoneError::unknown(format!("invalid response json: {}", err)))?;
        if let Some(err) = classify_response_error(&json) {
            return Err(err.with_context("publish response"));
        }
        let tid = json
            .get("tid")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(tid)
    }

    async fn upload_image(&self, image: &[u8]) -> Result<Value, QzoneError> {
        let cookie_header = build_cookie_header(&self.cookies);
        let skey = self
            .cookies
            .get("skey")
            .or_else(|| self.cookies.get("p_skey"))
            .ok_or_else(|| QzoneError::account("missing skey"))?
            .clone();
        let p_skey = self
            .cookies
            .get("p_skey")
            .ok_or_else(|| QzoneError::account("missing p_skey"))?
            .clone();
        let picfile = STANDARD.encode(image);
        let mut form: HashMap<&str, String> = HashMap::new();
        form.insert("filename", "filename".to_string());
        form.insert("zzpanelkey", "".to_string());
        form.insert("uploadtype", "1".to_string());
        form.insert("albumtype", "7".to_string());
        form.insert("exttype", "0".to_string());
        form.insert("skey", skey);
        form.insert("zzpaneluin", self.uin.to_string());
        form.insert("p_uin", self.uin.to_string());
        form.insert("uin", self.uin.to_string());
        form.insert("p_skey", p_skey);
        form.insert("output_type", "json".to_string());
        form.insert("qzonetoken", "".to_string());
        form.insert("refer", "shuoshuo".to_string());
        form.insert("charset", "utf-8".to_string());
        form.insert("output_charset", "utf-8".to_string());
        form.insert("upload_hd", "1".to_string());
        form.insert("hd_width", "2048".to_string());
        form.insert("hd_height", "10000".to_string());
        form.insert("hd_quality", "96".to_string());
        form.insert(
            "backUrls",
            "http://upbak.photo.qzone.qq.com/cgi-bin/upload/cgi_upload_image,http://119.147.64.75/cgi-bin/upload/cgi_upload_image".to_string(),
        );
        form.insert(
            "url",
            format!("{}?g_tk={}", UPLOAD_IMAGE_URL, self.gtk),
        );
        form.insert("base64", "1".to_string());
        form.insert("picfile", picfile);

        let res = self
            .client
            .post(UPLOAD_IMAGE_URL)
            .header("referer", format!("https://user.qzone.qq.com/{}", self.uin))
            .header("origin", "https://user.qzone.qq.com")
            .header("cookie", cookie_header)
            .form(&form)
            .send()
            .await
            .map_err(|err| classify_reqwest_error("upload image request", err))?;

        if !res.status().is_success() {
            return Err(classify_http_status("upload image http status", res.status().as_u16()));
        }
        let body = res
            .text()
            .await
            .map_err(|err| classify_reqwest_error("upload image read body", err))?;
        let start =
            body.find('{')
                .ok_or_else(|| QzoneError::unknown("invalid upload response"))?;
        let end =
            body.rfind('}')
                .ok_or_else(|| QzoneError::unknown("invalid upload response"))?;
        let json_str = &body[start..=end];
        let json: Value = serde_json::from_str(json_str)
            .map_err(|err| QzoneError::unknown(format!("invalid upload json: {}", err)))?;
        if let Some(err) = classify_response_error(&json) {
            return Err(err.with_context("upload response"));
        }
        Ok(json)
    }

    async fn fetch_image_bytes(&self, source: &str) -> Result<Vec<u8>, QzoneError> {
        if let Some(encoded) = source.strip_prefix("base64://") {
            return STANDARD
                .decode(encoded)
                .map_err(|err| QzoneError::unknown(format!("invalid base64 image: {}", err)));
        }
        if let Some(path) = source.strip_prefix("file://") {
            return fs::read(path)
                .map_err(|err| QzoneError::unknown(format!("read file failed: {}", err)));
        }
        if source.starts_with("http://") || source.starts_with("https://") {
            let res = self
                .client
                .get(source)
                .send()
                .await
                .map_err(|err| classify_reqwest_error("download image", err))?;
            if !res.status().is_success() {
                return Err(classify_http_status(
                    "download image http status",
                    res.status().as_u16(),
                ));
            }
            let bytes = res
                .bytes()
                .await
                .map_err(|err| classify_reqwest_error("read image bytes", err))?;
            return Ok(bytes.to_vec());
        }
        if Path::new(source).exists() {
            return fs::read(source)
                .map_err(|err| QzoneError::unknown(format!("read file failed: {}", err)));
        }
        Err(QzoneError::unknown("unsupported image source"))
    }
}

fn draft_to_text(draft: &Draft) -> String {
    let mut parts = Vec::new();
    for block in &draft.blocks {
        match block {
            DraftBlock::Paragraph { text } => {
                if !text.trim().is_empty() {
                    parts.push(text.trim().to_string());
                }
            }
            DraftBlock::Attachment { kind, .. } => {
                let label = match kind {
                    MediaKind::Image => None,
                    MediaKind::Video => Some("[视频]"),
                    MediaKind::File => Some("[文件]"),
                    MediaKind::Audio => Some("[语音]"),
                    MediaKind::Other => Some("[附件]"),
                };
                if let Some(label) = label {
                    parts.push(label.to_string());
                }
            }
        }
    }
    parts.join("\n\n")
}

fn parse_cookie_string(cookie_header: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut iter = trimmed.splitn(2, '=');
        let key = iter.next().unwrap_or("").trim();
        let value = iter.next().unwrap_or("").trim();
        if !key.is_empty() {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

fn build_cookie_header(cookies: &HashMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ")
}

fn generate_gtk(skey: &str) -> String {
    let mut hash_val: i64 = 5381;
    for ch in skey.chars() {
        hash_val = hash_val.saturating_add((hash_val << 5).saturating_add(ch as i64));
    }
    (hash_val & 0x7fffffff).to_string()
}

fn retry_delay_ms(kind: QzoneErrorKind, attempt: u32) -> TimestampMs {
    let (base, max) = match kind {
        QzoneErrorKind::Network => (5_000i64, 60_000i64),
        QzoneErrorKind::RiskControl => (60_000i64, 1_800_000i64),
        QzoneErrorKind::Account => (600_000i64, 3_600_000i64),
        QzoneErrorKind::Unknown => (60_000i64, 600_000i64),
    };
    let shift = attempt.saturating_sub(1).min(10);
    let delay = base.saturating_mul(1_i64 << shift);
    delay.min(max)
}

async fn get_cookies(
    state: &Arc<Mutex<QzoneState>>,
    napcat: &NapCatConfig,
) -> Result<HashMap<String, String>, QzoneError> {
    let now = now_ms();
    if let Some(cache) = state.lock().await.cookie_cache.as_ref() {
        if now.saturating_sub(cache.fetched_at_ms) < 300_000 {
            return Ok(cache.cookies.clone());
        }
    }

    let cookies = fetch_cookies_ws(napcat).await?;
    let mut guard = state.lock().await;
    guard.cookie_cache = Some(CookieCache {
        cookies: cookies.clone(),
        fetched_at_ms: now,
    });
    Ok(cookies)
}

async fn fetch_cookies_ws(napcat: &NapCatConfig) -> Result<HashMap<String, String>, QzoneError> {
    let ws_url = build_ws_url(&napcat.ws_url, napcat.access_token.as_deref());
    let mut request = ws_url
        .into_client_request()
        .map_err(|err| QzoneError::network(format!("invalid napcat ws url: {}", err)))?;
    if let Some(token) = napcat.access_token.as_deref() {
        let header_value = format!("Bearer {}", token);
        if let Ok(value) = header_value.parse() {
            request.headers_mut().insert("Authorization", value);
        }
    }

    let (ws_stream, _) = connect_async(request)
        .await
        .map_err(|err| QzoneError::network(format!("napcat ws connect failed: {}", err)))?;
    let (mut ws_write, mut ws_read) = ws_stream.split();
    let echo = format!("echo-{}", now_ms());
    let payload = serde_json::json!({
        "action": "get_cookies",
        "params": { "domain": "qzone.qq.com" },
        "echo": echo,
    });
    ws_write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            payload.to_string(),
        ))
        .await
        .map_err(|err| QzoneError::network(format!("napcat ws send failed: {}", err)))?;

    let response = timeout(Duration::from_secs(10), async {
        while let Some(msg) = ws_read.next().await {
            let msg = msg.map_err(|err| QzoneError::network(format!("napcat ws read failed: {}", err)))?;
            if !msg.is_text() {
                continue;
            }
            let value: Value = serde_json::from_str(msg.to_text().unwrap_or("{}"))
                .map_err(|err| QzoneError::network(format!("invalid napcat response: {}", err)))?;
            let Some(resp_echo) = value.get("echo").and_then(|v| v.as_str()) else {
                continue;
            };
            if resp_echo != echo {
                continue;
            }
            let cookie_str = value
                .get("data")
                .and_then(|data| data.get("cookies"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| QzoneError::network("napcat response missing cookies"))?;
            return Ok(cookie_str.to_string());
        }
        Err(QzoneError::network("napcat ws closed"))
    })
    .await
    .map_err(|_| QzoneError::network("napcat ws timeout"))??;

    Ok(parse_cookie_string(&response))
}

fn build_ws_url(base: &str, token: Option<&str>) -> String {
    if let Some(token) = token {
        if base.contains('?') {
            format!("{}&access_token={}", base, token)
        } else {
            format!("{}?access_token={}", base, token)
        }
    } else {
        base.to_string()
    }
}

fn now_ms() -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    now.as_millis() as i64
}

fn classify_reqwest_error(context: &str, err: reqwest::Error) -> QzoneError {
    if err.is_timeout() || err.is_connect() {
        return QzoneError::network(format!("{}: {}", context, err));
    }
    QzoneError::network(format!("{}: {}", context, err))
}

fn classify_http_status(context: &str, status: u16) -> QzoneError {
    let kind = match status {
        401 | 403 => QzoneErrorKind::Account,
        429 => QzoneErrorKind::RiskControl,
        500..=599 => QzoneErrorKind::Network,
        _ => QzoneErrorKind::Unknown,
    };
    QzoneError::new(kind, format!("{}: http {}", context, status))
}

fn classify_response_error(json: &Value) -> Option<QzoneError> {
    let ret = json.get("ret").and_then(|v| v.as_i64());
    if let Some(ret) = ret {
        if ret != 0 {
            let message = json
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("qzone error");
            return Some(classify_text_error(message));
        }
    }
    None
}

fn classify_text_error(message: &str) -> QzoneError {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("cookie")
        || lowered.contains("p_skey")
        || lowered.contains("skey")
        || lowered.contains("login")
        || lowered.contains("auth")
        || message.contains("登录")
    {
        return QzoneError::account(message);
    }
    if lowered.contains("risk")
        || lowered.contains("limit")
        || lowered.contains("频繁")
        || lowered.contains("风控")
        || lowered.contains("安全")
    {
        return QzoneError::risk(message);
    }
    if lowered.contains("timeout") || lowered.contains("connect") || lowered.contains("network") {
        return QzoneError::network(message);
    }
    QzoneError::unknown(message)
}

async fn collect_images(client: &QzoneClient, draft: &Draft) -> Result<Vec<Vec<u8>>, QzoneError> {
    let mut images = Vec::new();
    for block in &draft.blocks {
        if let DraftBlock::Attachment {
            kind: MediaKind::Image,
            reference,
        } = block
        {
            let bytes = match reference {
                MediaReference::RemoteUrl { url } => client.fetch_image_bytes(url).await?,
                MediaReference::Blob { .. } => {
                    return Err(QzoneError::unknown("blob media not supported in qzone sender"))
                }
            };
            images.push(bytes);
        }
    }
    Ok(images)
}

fn get_picbo_and_richval(upload: &Value) -> Result<(String, String), QzoneError> {
    let ret = upload.get("ret").and_then(|v| v.as_i64()).unwrap_or(-1);
    if ret != 0 {
        let message = upload
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("upload image failed");
        return Err(classify_text_error(message));
    }
    let data = upload
        .get("data")
        .ok_or_else(|| QzoneError::unknown("upload response missing data"))?;
    let url = data
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| QzoneError::unknown("upload response missing url"))?;
    let picbo = url
        .split("&bo=")
        .nth(1)
        .ok_or_else(|| QzoneError::unknown("upload response missing picbo"))?
        .to_string();

    let albumid = field_to_string(data, "albumid")?;
    let lloc = field_to_string(data, "lloc")?;
    let sloc = field_to_string(data, "sloc")?;
    let kind = field_to_string(data, "type")?;
    let height = field_to_string(data, "height")?;
    let width = field_to_string(data, "width")?;

    let richval = format!(
        ",{},{},{},{},{},{},,{},{}",
        albumid, lloc, sloc, kind, height, width, height, width
    );
    Ok((picbo, richval))
}

fn field_to_string(data: &Value, key: &str) -> Result<String, QzoneError> {
    let value = data
        .get(key)
        .ok_or_else(|| QzoneError::unknown(format!("upload response missing {}", key)))?;
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(QzoneError::unknown(format!("invalid field {}", key))),
    }
}
