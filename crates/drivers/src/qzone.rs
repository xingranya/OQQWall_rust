use std::collections::HashMap;
use std::fs;
#[cfg(debug_assertions)]
use std::io::{Read, Write};
#[cfg(debug_assertions)]
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use oqqwall_rust_core::draft::{
    Draft, DraftBlock, IngressMessage, MediaKind, MediaReference,
};
use oqqwall_rust_core::event::{
    BlobEvent, DraftEvent, Event, IngressEvent, MediaEvent, RenderEvent, SendEvent,
};
use oqqwall_rust_core::ids::{BlobId, IngressId, PostId, TimestampMs};
use oqqwall_rust_core::{build_draft_from_messages, derive_blob_id, Command};
use reqwest::Client;
#[cfg(debug_assertions)]
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

use crate::napcat::NapCatConfig;

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

const EMOTION_PUBLISH_URL: &str =
    "https://user.qzone.qq.com/proxy/domain/taotao.qzone.qq.com/cgi-bin/emotion_cgi_publish_v6";
const UPLOAD_IMAGE_URL: &str = "https://up.qzone.qq.com/cgi-bin/upload/cgi_upload_image";
#[cfg(debug_assertions)]
const EMUQZONE_PORT: u16 = 18080;
#[cfg(debug_assertions)]
const EMUQZONE_MAX_POSTS: usize = 50;

#[derive(Debug, Clone)]
pub struct QzoneRuntimeConfig {
    pub napcat_by_group: HashMap<String, NapCatConfig>,
    pub default_napcat: Option<NapCatConfig>,
    #[cfg(debug_assertions)]
    pub use_virt_qzone: bool,
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
    blob_paths: HashMap<BlobId, String>,
    ingress_messages: HashMap<IngressId, IngressMessage>,
    post_ingress: HashMap<PostId, Vec<IngressId>>,
    render_blobs: HashMap<PostId, RenderBlobs>,
}

impl QzoneState {
    fn register_media_reference(&mut self, ingress_id: IngressId, idx: usize, blob_id: BlobId) {
        if let Some(message) = self.ingress_messages.get_mut(&ingress_id) {
            if let Some(attachment) = message.attachments.get_mut(idx) {
                attachment.reference = MediaReference::Blob { blob_id };
            }
        }
    }
}

struct CookieCache {
    cookies: HashMap<String, String>,
    fetched_at_ms: TimestampMs,
}

#[derive(Debug, Default, Clone, Copy)]
struct RenderBlobs {
    png: Option<BlobId>,
}

#[cfg(debug_assertions)]
#[derive(Default)]
struct EmuQzoneState {
    posts: Vec<EmuQzonePost>,
    next_id: u64,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Serialize)]
struct EmuQzonePost {
    id: u64,
    timestamp_ms: TimestampMs,
    text: String,
    images: Vec<String>,
    image_count: usize,
    status: String,
}

pub fn spawn_qzone_sender(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    runtime: QzoneRuntimeConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug_log!(
            "qzone sender start: groups={} default_napcat={}",
            runtime.napcat_by_group.len(),
            runtime.default_napcat.is_some()
        );
        #[cfg(debug_assertions)]
        debug_log!("qzone sender mode: use_virt_qzone={}", runtime.use_virt_qzone);
        #[cfg(debug_assertions)]
        let emu_state = if runtime.use_virt_qzone {
            let state = Arc::new(std::sync::Mutex::new(EmuQzoneState::default()));
            spawn_emuqzone_server(state.clone());
            Some(state)
        } else {
            None
        };
        let state = Arc::new(Mutex::new(QzoneState::default()));
        let mut bus_rx = bus_rx;

        loop {
            let env = match bus_rx.recv().await {
                Ok(env) => env,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            match env.event {
                Event::Ingress(IngressEvent::MessageAccepted {
                    ingress_id, message, ..
                }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.insert(ingress_id, message);
                }
                Event::Ingress(IngressEvent::MessageIgnored { ingress_id, .. }) => {
                    let mut guard = state.lock().await;
                    guard.ingress_messages.remove(&ingress_id);
                }
                Event::Media(MediaEvent::MediaFetchSucceeded {
                    ingress_id,
                    attachment_index,
                    blob_id,
                }) => {
                    let mut guard = state.lock().await;
                    guard.register_media_reference(ingress_id, attachment_index, blob_id);
                }
                Event::Draft(DraftEvent::PostDraftCreated {
                    post_id,
                    draft,
                    ingress_ids,
                    ..
                }) => {
                    debug_log!(
                        "qzone draft cached: post_id={} blocks={}",
                        post_id.0,
                        draft.blocks.len()
                    );
                    let mut guard = state.lock().await;
                    guard.drafts.insert(post_id, draft);
                    guard.post_ingress.insert(post_id, ingress_ids);
                }
                Event::Blob(BlobEvent::BlobPersisted { blob_id, path }) => {
                    let mut guard = state.lock().await;
                    guard.blob_paths.insert(blob_id, path);
                }
                Event::Blob(BlobEvent::BlobReleased { blob_id })
                | Event::Blob(BlobEvent::BlobGcRequested { blob_id }) => {
                    let mut guard = state.lock().await;
                    guard.blob_paths.remove(&blob_id);
                }
                Event::Render(RenderEvent::PngReady { post_id, blob_id }) => {
                    let mut guard = state.lock().await;
                    let entry = guard.render_blobs.entry(post_id).or_default();
                    entry.png = Some(blob_id);
                }
                Event::Render(RenderEvent::RenderFailed { post_id, .. }) => {
                    let mut guard = state.lock().await;
                    let entry = guard.render_blobs.entry(post_id).or_default();
                    entry.png = None;
                }
                Event::Send(SendEvent::SendStarted {
                    post_id,
                    group_id,
                    account_id,
                    started_at_ms,
                    ..
                }) => {
                    debug_log!(
                        "qzone send started: post_id={} group_id={} account_id={} started_at_ms={}",
                        post_id.0,
                        group_id,
                        account_id,
                        started_at_ms
                    );
                    #[cfg(debug_assertions)]
                    if runtime.use_virt_qzone {
                        let Some(emu_state) = emu_state.as_ref() else {
                            debug_log!("emuqzone send failed: missing emulator state");
                            let err = QzoneError::unknown("missing emuqzone state");
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
                        let (draft, blob_paths, preview_blobs) = {
                            let guard = state.lock().await;
                            (
                                resolve_draft_for_send(&guard, post_id),
                                guard.blob_paths.clone(),
                                render_preview_blobs(&guard, post_id),
                            )
                        };
                        let Some(draft) = draft else {
                            debug_log!("emuqzone send failed: missing draft post_id={}", post_id.0);
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
                        if preview_blobs.is_empty() {
                            debug_log!(
                                "emuqzone send failed: missing render preview post_id={}",
                                post_id.0
                            );
                            let err = QzoneError::unknown("missing render preview");
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
                        let content = draft_to_text(&draft);
                        let images =
                            match collect_emuqzone_images(&draft, &blob_paths, &preview_blobs) {
                            Ok(images) => images,
                            Err(err) => {
                                debug_log!(
                                    "emuqzone collect images failed: kind={:?} message={}",
                                    err.kind,
                                    err.message
                                );
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
                        debug_log!(
                            "emuqzone publish attempt: post_id={} attempt={} images={} content_len={}",
                            post_id.0,
                            attempt,
                            images.len(),
                            content.len()
                        );
                        append_emuqzone_post(emu_state, content, images);
                        debug_log!("emuqzone publish success: post_id={}", post_id.0);
                        let mut guard = state.lock().await;
                        guard.attempts.remove(&post_id);
                        let event = SendEvent::SendSucceeded {
                            post_id,
                            account_id,
                            finished_at_ms: started_at_ms,
                            remote_id: None,
                        };
                        let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                        continue;
                    }
                    let napcat = runtime
                        .napcat_by_group
                        .get(&group_id)
                        .or_else(|| runtime.default_napcat.as_ref());
                    let Some(napcat) = napcat else {
                        debug_log!(
                            "qzone send failed: missing napcat config for group_id={}",
                            group_id
                        );
                        let event = SendEvent::SendFailed {
                            post_id,
                            account_id,
                            attempt: 1,
                            retry_at_ms: started_at_ms.saturating_add(30_000),
                            error: "missing napcat config for group".to_string(),
                        };
                        let _ = cmd_tx.send(Command::DriverEvent(Event::Send(event))).await;
                        continue;
                    };
                    let cookies = match get_cookies(&state, napcat).await {
                        Ok(cookies) => cookies,
                        Err(err) => {
                            debug_log!(
                                "qzone get cookies failed: kind={:?} message={}",
                                err.kind,
                                err.message
                            );
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
                            debug_log!(
                                "qzone client init failed: kind={:?} message={}",
                                err.kind,
                                err.message
                            );
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
                    let (draft, blob_paths, preview_blobs) = {
                        let guard = state.lock().await;
                        (
                            resolve_draft_for_send(&guard, post_id),
                            guard.blob_paths.clone(),
                            render_preview_blobs(&guard, post_id),
                        )
                    };
                    let Some(draft) = draft else {
                        debug_log!("qzone send failed: missing draft post_id={}", post_id.0);
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
                    if preview_blobs.is_empty() {
                        debug_log!(
                            "qzone send failed: missing render preview post_id={}",
                            post_id.0
                        );
                        let err = QzoneError::unknown("missing render preview");
                        let retry_at =
                            started_at_ms.saturating_add(retry_delay_ms(err.kind, 1));
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

                    let content = draft_to_text(&draft);
                    let images =
                        match collect_images(&draft, &blob_paths, &preview_blobs).await
                        {
                        Ok(images) => images,
                        Err(err) => {
                            debug_log!(
                                "qzone collect images failed: kind={:?} message={}",
                                err.kind,
                                err.message
                            );
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
                    debug_log!(
                        "qzone publish attempt: post_id={} attempt={} images={} content_len={}",
                        post_id.0,
                        attempt,
                        images.len(),
                        content.len()
                    );

                    match client.publish_emotion(&content, &images).await {
                        Ok(tid) => {
                            debug_log!(
                                "qzone publish success: post_id={} tid={}",
                                post_id.0,
                                tid
                            );
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
                            debug_log!(
                                "qzone publish failed: post_id={} attempt={} kind={:?} message={}",
                                post_id.0,
                                attempt,
                                err.kind,
                                err.message
                            );
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
        debug_log!("qzone client init: cookie_count={}", cookies.len());
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
        debug_log!("qzone client init: uin={}", uin);
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
        debug_log!(
            "qzone publish request: content_len={} images={}",
            content.len(),
            images.len()
        );
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
        debug_log!("qzone upload image: size_bytes={}", image.len());
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

fn resolve_draft_for_send(state: &QzoneState, post_id: PostId) -> Option<Draft> {
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        let mut messages = Vec::new();
        for ingress_id in ingress_ids {
            let Some(message) = state.ingress_messages.get(ingress_id) else {
                return state.drafts.get(&post_id).cloned();
            };
            messages.push(message.clone());
        }
        if !messages.is_empty() {
            return Some(build_draft_from_messages(&messages));
        }
    }
    state.drafts.get(&post_id).cloned()
}

fn render_preview_blobs(state: &QzoneState, post_id: PostId) -> Vec<BlobId> {
    let Some(render) = state.render_blobs.get(&post_id) else {
        return Vec::new();
    };
    if let Some(png) = render.png {
        return vec![png];
    }
    Vec::new()
}

fn resolve_blob_path(
    blob_paths: &HashMap<BlobId, String>,
    blob_id: BlobId,
) -> Result<String, QzoneError> {
    blob_paths
        .get(&blob_id)
        .cloned()
        .ok_or_else(|| QzoneError::unknown("missing blob path"))
}

fn resolve_local_image_path(
    kind: MediaKind,
    reference: &MediaReference,
    blob_paths: &HashMap<BlobId, String>,
) -> Result<String, QzoneError> {
    match reference {
        MediaReference::Blob { blob_id } => resolve_blob_path(blob_paths, *blob_id),
        MediaReference::RemoteUrl { url } => resolve_remote_image_path(kind, url),
    }
}

fn resolve_remote_image_path(kind: MediaKind, source: &str) -> Result<String, QzoneError> {
    if let Some(path) = source.strip_prefix("file://") {
        return Ok(path.to_string());
    }
    if Path::new(source).exists() {
        return Ok(source.to_string());
    }
    if let Some((mime, bytes)) = decode_inline_source(source)? {
        let ext = mime
            .as_deref()
            .and_then(ext_from_content_type)
            .unwrap_or_else(|| default_ext_for_kind(kind));
        return cache_inline_bytes(&bytes, ext);
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return Err(QzoneError::unknown(
            "remote http image url not allowed; require local file",
        ));
    }
    Err(QzoneError::unknown("unsupported image source"))
}

fn decode_inline_source(source: &str) -> Result<Option<(Option<String>, Vec<u8>)>, QzoneError> {
    if let Some(payload) = source.strip_prefix("data:") {
        let (meta, data) = payload
            .split_once(',')
            .ok_or_else(|| QzoneError::unknown("invalid data url"))?;
        let mime = meta
            .split(';')
            .next()
            .and_then(|value| (!value.is_empty()).then_some(value.to_string()));
        let bytes = if meta.contains(";base64") {
            STANDARD
                .decode(data)
                .map_err(|err| QzoneError::unknown(format!("invalid data url base64: {}", err)))?
        } else {
            data.as_bytes().to_vec()
        };
        return Ok(Some((mime, bytes)));
    }
    if let Some(encoded) = source.strip_prefix("base64://") {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|err| QzoneError::unknown(format!("invalid base64 image: {}", err)))?;
        return Ok(Some((None, bytes)));
    }
    Ok(None)
}

fn ext_from_content_type(content_type: &str) -> Option<&'static str> {
    let base = content_type.split(';').next()?.trim().to_ascii_lowercase();
    match base.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}

fn default_ext_for_kind(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "png",
        MediaKind::Video => "mp4",
        MediaKind::Audio => "mp3",
        MediaKind::File | MediaKind::Other => "bin",
    }
}

fn cache_inline_bytes(bytes: &[u8], ext: &str) -> Result<String, QzoneError> {
    let ext = ext.trim_start_matches('.');
    let parts: Vec<&[u8]> = vec![bytes, ext.as_bytes()];
    let blob_id = derive_blob_id(&parts);
    let dir = inline_cache_root();
    fs::create_dir_all(&dir)
        .map_err(|err| QzoneError::unknown(format!("create inline dir failed: {}", err)))?;
    let filename = format!("{}.{}", id128_hex(blob_id.0), ext);
    let path = dir.join(filename);
    if !path.exists() {
        fs::write(&path, bytes)
            .map_err(|err| QzoneError::unknown(format!("write inline image failed: {}", err)))?;
    }
    Ok(path.to_string_lossy().to_string())
}

fn inline_cache_root() -> PathBuf {
    std::env::var("OQQWALL_BLOB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/blobs"))
        .join("inline")
}

fn read_local_image_bytes(path: &str) -> Result<Vec<u8>, QzoneError> {
    let path = path.strip_prefix("file://").unwrap_or(path);
    fs::read(path).map_err(|err| QzoneError::unknown(format!("read file failed: {}", err)))
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

#[cfg(debug_assertions)]
fn collect_emuqzone_images(
    draft: &Draft,
    blob_paths: &HashMap<BlobId, String>,
    preview_blobs: &[BlobId],
) -> Result<Vec<String>, QzoneError> {
    let mut images = Vec::new();
    for blob_id in preview_blobs {
        let path = resolve_blob_path(blob_paths, *blob_id)?;
        images.push(process_emuqzone_image(&path));
    }
    for block in &draft.blocks {
        if let DraftBlock::Attachment {
            kind: MediaKind::Image,
            reference,
        } = block
        {
            let path = resolve_local_image_path(MediaKind::Image, reference, blob_paths)?;
            images.push(process_emuqzone_image(&path));
        }
    }
    Ok(images)
}

#[cfg(debug_assertions)]
fn append_emuqzone_post(
    state: &Arc<std::sync::Mutex<EmuQzoneState>>,
    text: String,
    images: Vec<String>,
) {
    let timestamp_ms = now_ms();
    let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
    let post = EmuQzonePost {
        id: guard.next_id,
        timestamp_ms,
        text,
        image_count: images.len(),
        images,
        status: "success".to_string(),
    };
    guard.next_id = guard.next_id.saturating_add(1);
    guard.posts.push(post);
    if guard.posts.len() > EMUQZONE_MAX_POSTS {
        let drain = guard.posts.len().saturating_sub(EMUQZONE_MAX_POSTS);
        guard.posts.drain(0..drain);
    }
}

#[cfg(debug_assertions)]
fn process_emuqzone_image(source: &str) -> String {
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("data:image")
    {
        return source.to_string();
    }
    if let Some(path) = source.strip_prefix("file://") {
        if let Ok(bytes) = fs::read(path) {
            let mime = mime_from_path(path).unwrap_or("image/jpeg");
            return format!("data:{};base64,{}", mime, STANDARD.encode(bytes));
        }
    }
    if Path::new(source).exists() {
        if let Ok(bytes) = fs::read(source) {
            let mime = mime_from_path(source).unwrap_or("image/jpeg");
            return format!("data:{};base64,{}", mime, STANDARD.encode(bytes));
        }
    }
    if STANDARD.decode(source).is_ok() {
        return format!("data:image/jpeg;base64,{}", source);
    }
    source.to_string()
}

#[cfg(debug_assertions)]
fn mime_from_path(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" | "svgz" => Some("image/svg+xml"),
        _ => None,
    }
}

#[cfg(debug_assertions)]
fn spawn_emuqzone_server(state: Arc<std::sync::Mutex<EmuQzoneState>>) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", EMUQZONE_PORT)) {
            Ok(listener) => listener,
            Err(err) => {
                debug_log!("emuqzone server bind failed: {}", err);
                return;
            }
        };
        debug_log!(
            "emuqzone server listening: http://127.0.0.1:{}",
            EMUQZONE_PORT
        );
        for stream in listener.incoming() {
            let state = state.clone();
            match stream {
                Ok(stream) => {
                    std::thread::spawn(move || {
                        handle_emuqzone_conn(stream, state);
                    });
                }
                Err(err) => {
                    debug_log!("emuqzone accept failed: {}", err);
                    break;
                }
            }
        }
    });
}

#[cfg(debug_assertions)]
fn handle_emuqzone_conn(
    mut stream: TcpStream,
    state: Arc<std::sync::Mutex<EmuQzoneState>>,
) {
    let mut buffer = [0u8; 2048];
    let Ok(size) = stream.read(&mut buffer) else {
        return;
    };
    if size == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buffer[..size]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    match path {
        "/" => {
            let body = emuqzone_html();
            write_http_response(&mut stream, 200, "text/html; charset=utf-8", body.as_bytes());
        }
        "/data" => {
            let body = {
                let guard = state.lock().unwrap_or_else(|err| err.into_inner());
                serde_json::to_vec(&guard.posts).unwrap_or_else(|_| b"[]".to_vec())
            };
            write_http_response(&mut stream, 200, "application/json; charset=utf-8", &body);
        }
        _ => {
            write_http_response(&mut stream, 404, "text/plain; charset=utf-8", b"Not Found");
        }
    }
}

#[cfg(debug_assertions)]
fn write_http_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        status,
        status_text,
        content_type,
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

#[cfg(debug_assertions)]
fn emuqzone_html() -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>EmuQzone</title>
<style>
body {{ font-family: Arial, sans-serif; margin: 20px; }}
.post {{ border: 1px solid #ddd; margin: 10px 0; padding: 15px; border-radius: 6px; }}
.post-header {{ font-weight: bold; margin-bottom: 8px; }}
.post-content {{ white-space: pre-wrap; margin-bottom: 8px; }}
.post-images {{ display: flex; gap: 10px; flex-wrap: wrap; }}
.post-image {{ max-width: 220px; max-height: 220px; border-radius: 4px; }}
.refresh-btn {{ padding: 8px 14px; background: #1e88e5; color: white; border: none; border-radius: 4px; cursor: pointer; }}
</style>
</head>
<body>
<h1>EmuQzone</h1>
<p>Listening on http://127.0.0.1:{}</p>
<button class="refresh-btn" onclick="loadData()">Refresh</button>
<div id="posts"></div>
<script>
function loadData() {{
  fetch('/data')
    .then(r => r.json())
    .then(data => {{
      const posts = document.getElementById('posts');
      if (!data.length) {{
        posts.innerHTML = '<p>No posts yet.</p>';
        return;
      }}
      posts.innerHTML = data.map(post => `
        <div class="post">
          <div class="post-header">Post #${{post.id}} - ${{post.timestamp_ms}}</div>
          <div class="post-content">${{post.text || ''}}</div>
          ${{post.images && post.images.length ? `
            <div class="post-images">
              ${{post.images.map(img => `<img src="${{img}}" alt="image" class="post-image">`).join('')}}
            </div>` : ''}}
          <div>Status: ${{post.status}} | Images: ${{post.image_count || 0}}</div>
        </div>
      `).join('');
    }})
    .catch(() => {{
      document.getElementById('posts').innerHTML = '<p>Failed to load data.</p>';
    }});
}}
loadData();
setInterval(loadData, 5000);
</script>
</body>
</html>"#,
        EMUQZONE_PORT
    )
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
            debug_log!(
                "qzone cookies cache hit: age_ms={}",
                now.saturating_sub(cache.fetched_at_ms)
            );
            return Ok(cache.cookies.clone());
        }
    }

    debug_log!("qzone cookies cache miss: fetching via napcat ws");
    let cookies = fetch_cookies_ws(napcat).await?;
    let mut guard = state.lock().await;
    guard.cookie_cache = Some(CookieCache {
        cookies: cookies.clone(),
        fetched_at_ms: now,
    });
    Ok(cookies)
}

async fn fetch_cookies_ws(napcat: &NapCatConfig) -> Result<HashMap<String, String>, QzoneError> {
    debug_log!(
        "qzone fetch cookies ws: ws_url={} token_present={}",
        ws_url_for_log(&napcat.ws_url),
        napcat.access_token.is_some()
    );
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
    debug_log!("qzone fetch cookies ws connected");
    let (mut ws_write, mut ws_read) = ws_stream.split();
    let echo = format!("echo-{}", now_ms());
    let payload = serde_json::json!({
        "action": "get_cookies",
        "params": { "domain": "qzone.qq.com" },
        "echo": echo,
    });
    debug_log!("qzone fetch cookies ws send request: echo={}", echo);
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
            debug_log!("qzone fetch cookies ws received response");
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

#[cfg(debug_assertions)]
fn ws_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
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

async fn collect_images(
    draft: &Draft,
    blob_paths: &HashMap<BlobId, String>,
    preview_blobs: &[BlobId],
) -> Result<Vec<Vec<u8>>, QzoneError> {
    let mut sources = Vec::new();
    for blob_id in preview_blobs {
        sources.push(resolve_blob_path(blob_paths, *blob_id)?);
    }
    for block in &draft.blocks {
        if let DraftBlock::Attachment {
            kind: MediaKind::Image,
            reference,
        } = block
        {
            sources.push(resolve_local_image_path(
                MediaKind::Image,
                reference,
                blob_paths,
            )?);
        }
    }

    let mut images = Vec::new();
    for source in sources {
        debug_log!("qzone collect image: file={}", source);
        images.push(read_local_image_bytes(&source)?);
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
