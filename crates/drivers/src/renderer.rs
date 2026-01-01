use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::fs;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crate::napcat::{extract_message_lite, NapCatConfig};
use oqqwall_rust_core::event::{BlobEvent, Event, RenderEvent};
use oqqwall_rust_core::{derive_blob_id, Command, Draft, DraftBlock, PostId, StateView};
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use skia_safe::canvas::SrcRectConstraint;
use skia_safe::textlayout::{
    FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle, TypefaceFontProvider,
};
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{
    image_filters, Canvas, ClipOp, Color4f, Data, EncodedImageFormat, FontStyle, Image, Paint,
    PathBuilder, Rect, RRect, SamplingOptions, Typeface,
};
use skia_safe::utils::OrderedFontMgr;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::sync::broadcast::error::RecvError;

mod embedded_resources {
    include!(concat!(env!("OUT_DIR"), "/embedded_resources.rs"));
}

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

const FORWARD_PREFIX: &str = "[合并转发:";
const MAX_FORWARD_DEPTH: u32 = 4;
const MEASURE_MAX_WIDTH: f32 = 10_000.0;
const EMOJI_FONT_ALIAS: &str = "OQQWall Emoji";
const FONT_FAMILIES: [&str; 3] = [
    "PingFang SC",
    EMOJI_FONT_ALIAS,
    "Apple Color Emoji",
];
const EMOJI_FONT_FAMILIES: [&str; 2] = [
    EMOJI_FONT_ALIAS,
    "Apple Color Emoji",
];
static FONT_BYTES_CACHE: OnceLock<Vec<FontBytes>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct RendererRuntimeConfig {
    pub blob_root: PathBuf,
    pub canvas_width_px: u32,
    pub max_height_px: u32,
    pub napcat_by_group: HashMap<String, NapCatConfig>,
    pub default_napcat: Option<NapCatConfig>,
}

impl Default for RendererRuntimeConfig {
    fn default() -> Self {
        let blob_root = std::env::var("OQQWALL_BLOB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/blobs"));
        Self {
            blob_root,
            canvas_width_px: 384,
            max_height_px: 2304,
            napcat_by_group: HashMap::new(),
            default_napcat: None,
        }
    }
}

#[derive(Debug, Clone)]
struct HeaderInfo {
    group_id: String,
    user_id: String,
    post_id_hex: String,
    sender_name: Option<String>,
}

#[derive(Debug, Clone)]
enum BlockKind {
    Text { lines: Vec<InlineLine> },
    Image { image: Option<ResolvedImage> },
    MediaCard {
        lines: Vec<String>,
        icon_text: String,
        media_kind: oqqwall_rust_core::MediaKind,
    },
    FileCard {
        name_lines: Vec<String>,
        meta_line: Option<String>,
        icon_text: String,
    },
}

#[derive(Debug, Clone)]
struct InlineLine {
    runs: Vec<InlineRun>,
    width: u32,
}

#[derive(Debug, Clone)]
enum InlineRun {
    Text(String),
    Face { id: String },
}

#[derive(Debug, Clone)]
enum InlineAtom {
    Char(char),
    Face(String),
}

#[derive(Debug, Clone)]
struct BlockLayout {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    kind: BlockKind,
}

#[derive(Debug, Clone)]
struct ResolvedImage {
    bytes: Option<Vec<u8>>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Clone)]
struct RenderImageSources {
    avatar: Option<ResolvedImage>,
    block_images: Vec<Option<ResolvedImage>>,
    block_labels: Vec<Option<String>>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct TextMeasureKey {
    font_size: u32,
    font_weight: u32,
    text: String,
}

#[derive(Debug)]
struct TextMeasurer {
    font_collection: FontCollection,
    cache: HashMap<TextMeasureKey, u32>,
}

impl ResolvedImage {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let size = image_size_from_bytes(&bytes);
        let (width, height) = size.map_or((None, None), |(w, h)| (Some(w), Some(h)));
        Self {
            bytes: Some(bytes),
            width,
            height,
        }
    }

    fn has_bytes(&self) -> bool {
        self.bytes.as_ref().map(|b| !b.is_empty()).unwrap_or(false)
    }
}

impl TextMeasurer {
    fn new(font_collection: FontCollection) -> Self {
        Self {
            font_collection,
            cache: HashMap::new(),
        }
    }

    fn measure_text_width(&mut self, text: &str, font_size: u32, font_weight: u32) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let key = TextMeasureKey {
            font_size,
            font_weight,
            text: text.to_string(),
        };
        if let Some(width) = self.cache.get(&key) {
            return *width;
        }
        let width_px = if let Some((paragraph, _)) = build_line_paragraph(
            &self.font_collection,
            text,
            font_size,
            font_weight,
            Color4f::new(0.0, 0.0, 0.0, 1.0),
        ) {
            paragraph.max_intrinsic_width().ceil().max(0.0) as u32
        } else {
            0
        };
        self.cache.insert(key, width_px);
        width_px
    }
}


pub fn spawn_renderer(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    config: RendererRuntimeConfig,
) -> JoinHandle<()> {
    let font_dir = resolve_font_dir();
    init_font_bytes_cache(&font_dir);
    let _ = emoji_png_store();
    tokio::spawn(async move {
        debug_log!(
            "renderer task start: blob_root={} canvas_width={} max_height={}",
            config.blob_root.display(),
            config.canvas_width_px,
            config.max_height_px
        );
        let mut state = StateView::default();
        let mut bus_rx = bus_rx;

        loop {
            let env = match bus_rx.recv().await {
                Ok(env) => env,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };

            state = state.reduce(&env);

            if let Event::Render(RenderEvent::RenderRequested {
                post_id,
                attempt,
                ..
            }) = env.event
            {
                if let Err(err) =
                    handle_render_request(&cmd_tx, &state, post_id, attempt, &config).await
                {
                    debug_log!("render failed: post_id={} err={}", post_id.0, err);
                }
            }
        }

        debug_log!("renderer task end");
    })
}

async fn handle_render_request(
    cmd_tx: &mpsc::Sender<Command>,
    state: &StateView,
    post_id: PostId,
    attempt: u32,
    config: &RendererRuntimeConfig,
) -> Result<(), String> {
    let draft = match state.drafts.get(&post_id) {
        Some(draft) => draft.clone(),
        None => {
            return send_render_failed(
                cmd_tx,
                post_id,
                attempt,
                "missing draft for render".to_string(),
            )
            .await;
        }
    };

    let header = extract_header(state, post_id);
    let draft = resolve_forward_draft(&draft, &header, config).await;
    let image_sources = resolve_image_sources(state, &draft, &header).await;
    let bytes = match render_png_async(&draft, &header, &image_sources, config).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return send_render_failed(cmd_tx, post_id, attempt, err).await;
        }
    };

    let blob_id = render_blob_id(post_id);
    let (path, size_bytes) = persist_blob(&config.blob_root, "png", "png", blob_id, &bytes)?;

    send_event(
        cmd_tx,
        Event::Blob(BlobEvent::BlobRegistered {
            blob_id,
            size_bytes,
        }),
    )
    .await?;
    send_event(
        cmd_tx,
        Event::Blob(BlobEvent::BlobPersisted {
            blob_id,
            path,
        }),
    )
    .await?;

    send_event(cmd_tx, Event::Render(RenderEvent::PngReady { post_id, blob_id })).await?;

    Ok(())
}

struct ForwardContext {
    client: Client,
    api_base: String,
    token: Option<String>,
    cache: HashMap<String, Vec<DraftBlock>>,
    seen: HashSet<String>,
}

async fn resolve_forward_draft(
    draft: &Draft,
    header: &HeaderInfo,
    config: &RendererRuntimeConfig,
) -> Draft {
    if !draft_has_forward(draft) {
        return draft.clone();
    }

    let Some(napcat) = napcat_config_for_group(config, &header.group_id) else {
        return draft.clone();
    };
    let Some(api_base) = napcat_http_base(&napcat.ws_url) else {
        return draft.clone();
    };
    let client = match Client::builder().timeout(Duration::from_secs(6)).build() {
        Ok(client) => client,
        Err(_) => return draft.clone(),
    };

    let mut context = ForwardContext {
        client,
        api_base,
        token: napcat.access_token.clone(),
        cache: HashMap::new(),
        seen: HashSet::new(),
    };

    let mut blocks = Vec::new();
    for block in &draft.blocks {
        match block {
            DraftBlock::Paragraph { text } => {
                let mut expanded = expand_forward_in_text(text, &mut context, 0).await;
                blocks.append(&mut expanded);
            }
            DraftBlock::Attachment { .. } => blocks.push(block.clone()),
        }
    }
    Draft { blocks }
}

fn draft_has_forward(draft: &Draft) -> bool {
    draft.blocks.iter().any(|block| match block {
        DraftBlock::Paragraph { text } => text.contains(FORWARD_PREFIX),
        _ => false,
    })
}

fn napcat_config_for_group<'a>(
    config: &'a RendererRuntimeConfig,
    group_id: &str,
) -> Option<&'a NapCatConfig> {
    config
        .napcat_by_group
        .get(group_id)
        .or_else(|| config.default_napcat.as_ref())
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

fn forward_placeholder(id: &str) -> String {
    if id.is_empty() {
        "[合并转发]".to_string()
    } else {
        format!("[合并转发:{}]", id)
    }
}

fn push_text_block(blocks: &mut Vec<DraftBlock>, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    blocks.push(DraftBlock::Paragraph {
        text: trimmed.to_string(),
    });
}

fn expand_forward_in_text<'a>(
    text: &'a str,
    context: &'a mut ForwardContext,
    depth: u32,
) -> Pin<Box<dyn Future<Output = Vec<DraftBlock>> + Send + 'a>> {
    Box::pin(async move {
        if depth >= MAX_FORWARD_DEPTH {
            let mut blocks = Vec::new();
            push_text_block(&mut blocks, text);
            return blocks;
        }

        let mut blocks = Vec::new();
        let mut remaining = text;
        while let Some(start) = remaining.find(FORWARD_PREFIX) {
            let (before, rest) = remaining.split_at(start);
            push_text_block(&mut blocks, before);

            let after_prefix = &rest[FORWARD_PREFIX.len()..];
            let Some(end) = after_prefix.find(']') else {
                push_text_block(&mut blocks, rest);
                return blocks;
            };
            let id = after_prefix[..end].trim();
            let mut resolved = forward_blocks_for_id(id, context, depth).await;
            blocks.append(&mut resolved);
            remaining = &after_prefix[end + 1..];
        }
        push_text_block(&mut blocks, remaining);
        blocks
    })
}

async fn forward_blocks_for_id(
    forward_id: &str,
    context: &mut ForwardContext,
    depth: u32,
) -> Vec<DraftBlock> {
    if forward_id.is_empty() || depth >= MAX_FORWARD_DEPTH {
        return vec![DraftBlock::Paragraph {
            text: forward_placeholder(forward_id),
        }];
    }

    if let Some(cached) = context.cache.get(forward_id) {
        return cached.clone();
    }
    if context.seen.contains(forward_id) {
        return vec![DraftBlock::Paragraph {
            text: forward_placeholder(forward_id),
        }];
    }
    context.seen.insert(forward_id.to_string());

    let resolved = match fetch_forward_messages(context, forward_id).await {
        Ok(messages) => forward_messages_to_blocks(&messages, context, depth + 1).await,
        Err(err) => {
            debug_log!("forward resolve failed: id={} err={}", forward_id, err);
            vec![DraftBlock::Paragraph {
                text: forward_placeholder(forward_id),
            }]
        }
    };
    context.cache.insert(forward_id.to_string(), resolved.clone());
    resolved
}

async fn fetch_forward_messages(
    context: &ForwardContext,
    forward_id: &str,
) -> Result<Vec<Value>, String> {
    let url = format!("{}/get_forward_msg", context.api_base);
    let mut req = context.client.post(url).json(&json!({ "message_id": forward_id }));
    if let Some(token) = context.token.as_ref() {
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

async fn forward_messages_to_blocks(
    messages: &[Value],
    context: &mut ForwardContext,
    depth: u32,
) -> Vec<DraftBlock> {
    let mut blocks = Vec::new();
    for message in messages {
        let payload = message
            .get("message")
            .or_else(|| message.get("content"));
        let (text, attachments) = extract_message_lite(payload);
        let mut text_blocks = expand_forward_in_text(&text, context, depth).await;
        blocks.append(&mut text_blocks);
        for attachment in attachments {
            blocks.push(DraftBlock::Attachment {
                kind: attachment.kind,
                reference: attachment.reference,
            });
        }
    }
    blocks
}

fn extract_header(state: &StateView, post_id: PostId) -> HeaderInfo {
    let mut group_id = "unknown".to_string();
    let mut user_id = "unknown".to_string();
    let mut sender_name = None;
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            if let Some(meta) = state.ingress_meta.get(ingress_id) {
                group_id = meta.group_id.clone();
                user_id = meta.user_id.clone();
                sender_name = meta.sender_name.clone().filter(|name| !name.trim().is_empty());
                break;
            }
        }
    }
    HeaderInfo {
        group_id,
        user_id,
        post_id_hex: id128_hex(post_id.0),
        sender_name,
    }
}

fn render_png(
    draft: &Draft,
    header: &HeaderInfo,
    image_sources: &RenderImageSources,
    config: &RendererRuntimeConfig,
) -> Result<Vec<u8>, String> {
    let padding = 20u32;
    let spacing_lg = 10u32;
    let spacing_xxl = 20u32;
    let bubble_pad_left = 8u32;
    let bubble_pad_right = 8u32;
    let bubble_pad_top = 6u32;
    let bubble_pad_bottom = 6u32;
    let font_size = 16u32;
    let line_height = 21u32;
    let face_size = 16u32;
    let title_size = 32u32;
    let meta_size = 12u32;
    let font_weight_title = 600u32;
    let font_weight_body = 400u32;
    let header_gap = 10u32;
    let avatar_size = 50u32;
    let radius_lg = 12u32;
    let card_padding = 8u32;
    let card_line_height = 18u32;
    let card_icon_size = 24u32;
    let card_icon_gap = 8u32;
    let file_padding = 7u32;
    let file_line_height = 18u32;
    let file_meta_height = 14u32;
    let file_meta_gap = 2u32;
    let file_icon_size = 40u32;
    let file_icon_gap = 6u32;
    let font_dir = resolve_font_dir();
    let font_collection = build_font_collection(&font_dir);
    let mut text_measurer = TextMeasurer::new(font_collection.clone());

    let scale = 4u32;
    debug_log!(
        "render start: blocks={} canvas_width={} max_height={} scale={}",
        draft.blocks.len(),
        config.canvas_width_px,
        config.max_height_px,
        scale
    );
    let content_width = config.canvas_width_px.saturating_sub(padding.saturating_mul(2));
    let header_x = padding;
    let header_y = padding;
    let header_text_x = header_x + avatar_size + header_gap;
    let header_text_width = content_width.saturating_sub(avatar_size + header_gap);

    let title_text = header.sender_name.clone().unwrap_or_else(|| {
        if header.user_id == "unknown" {
            "OQQWall".to_string()
        } else {
            format!("User {}", header.user_id)
        }
    });
    let title_text = truncate_text(
        &title_text,
        header_text_width,
        title_size,
        font_weight_title,
        &mut text_measurer,
    );
    let meta_source = if header.user_id == "unknown" {
        header.post_id_hex.clone()
    } else {
        header.user_id.clone()
    };
    let meta_text = truncate_text(
        &format!("QQ {}", meta_source),
        header_text_width,
        meta_size,
        font_weight_body,
        &mut text_measurer,
    );
    debug_log!(
        "render header: title={} meta={} header_text_width={}",
        title_text,
        meta_text,
        header_text_width
    );
    let title_y = header_y + title_size;
    let meta_y = title_y + meta_size + 4;
    let header_height = avatar_size.max(meta_y + meta_size - header_y);

    let mut cursor_y = header_y + header_height + spacing_xxl;
    let mut blocks = Vec::new();

    for (block_idx, block) in draft.blocks.iter().enumerate() {
        let layout = match block {
            DraftBlock::Paragraph { text } => {
                let max_text_w = content_width
                    .saturating_sub(bubble_pad_left + bubble_pad_right)
                    .max(1);
                let lines = wrap_inline_text(
                    text,
                    max_text_w,
                    font_size,
                    face_size,
                    font_weight_body,
                    &mut text_measurer,
                );
                let mut max_line_w = 0u32;
                for line in &lines {
                    max_line_w = max_line_w.max(line.width);
                }
                let bubble_w = (max_line_w + bubble_pad_left + bubble_pad_right)
                    .min(content_width)
                    .max(1);
                let height =
                    bubble_pad_top + bubble_pad_bottom + line_height * lines.len() as u32;
                BlockLayout {
                    x: padding,
                    y: cursor_y,
                    width: bubble_w,
                    height,
                    kind: BlockKind::Text { lines },
                }
            }
            DraftBlock::Attachment { kind, reference: _ } => {
                let image = image_sources
                    .block_images
                    .get(block_idx)
                    .and_then(|value| value.as_ref());
                let label_href = image_sources
                    .block_labels
                    .get(block_idx)
                    .and_then(|value| value.clone());
                match *kind {
                    oqqwall_rust_core::MediaKind::Image => {
                        let max_width = (content_width / 2).max(1);
                        let max_height = 300u32;
                        let (width, height) = match image
                            .and_then(|img| img.width.zip(img.height))
                            .filter(|(w, h)| *w > 0 && *h > 0)
                        {
                            Some((orig_w, orig_h)) => {
                                let scale_w = max_width as f32 / orig_w as f32;
                                let scale_h = max_height as f32 / orig_h as f32;
                                let scale = scale_w.min(scale_h).min(1.0);
                                let width = (orig_w as f32 * scale).round().max(1.0) as u32;
                                let height = (orig_h as f32 * scale).round().max(1.0) as u32;
                                (width, height)
                            }
                            None => {
                                let width = max_width;
                                let height = (width.saturating_mul(3).saturating_div(4))
                                    .min(max_height)
                                    .max(1);
                                (width, height)
                            }
                        };
                        BlockLayout {
                            x: padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::Image {
                                image: image.cloned(),
                            },
                        }
                    }
                    oqqwall_rust_core::MediaKind::File => {
                        let width = content_width.min(320).max(1);
                        let text_max_w = width
                            .saturating_sub(file_padding * 2 + file_icon_size + file_icon_gap)
                            .max(1);
                        let filename = label_href
                            .as_deref()
                            .and_then(extract_filename)
                            .unwrap_or_else(|| "Unknown file".to_string());
                        let mut name_lines = wrap_text(
                            &filename,
                            text_max_w,
                            font_size,
                            font_weight_body,
                            &mut text_measurer,
                        );
                        name_lines = limit_lines(
                            name_lines,
                            2,
                            text_max_w,
                            font_size,
                            font_weight_body,
                            &mut text_measurer,
                        );
                        let meta_line = Some("Size: unknown".to_string());
                        let name_height = name_lines.len() as u32 * file_line_height;
                        let meta_height = if meta_line.is_some() {
                            file_meta_height + file_meta_gap
                        } else {
                            0
                        };
                        let text_height = name_height + meta_height;
                        let content_height = file_icon_size.max(text_height);
                        let height = content_height + file_padding * 2;
                        BlockLayout {
                            x: padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::FileCard {
                                name_lines,
                                meta_line,
                                icon_text: file_icon_text(&filename),
                            },
                        }
                    }
                    _ => {
                        let width = content_width.min(320).max(1);
                        let height = 90u32;
                        let text_max_w = width
                            .saturating_sub(card_padding * 2 + card_icon_size + card_icon_gap)
                            .max(1);
                        let label = media_label(*kind);
                        let mut lines = vec![label.to_string()];
                        if let Some(detail) = label_href.as_deref().and_then(extract_filename) {
                            let detail_line = truncate_text(
                                &detail,
                                text_max_w,
                                font_size,
                                font_weight_body,
                                &mut text_measurer,
                            );
                            if !detail_line.is_empty() {
                                lines.push(detail_line);
                            }
                        }
                        BlockLayout {
                            x: padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::MediaCard {
                                lines,
                                icon_text: media_icon_text(*kind),
                                media_kind: *kind,
                            },
                        }
                    }
                }
            }
        };
        match &layout.kind {
            BlockKind::Text { lines } => {
                debug_log!(
                    "layout block: idx={} kind=text width={} height={} lines={}",
                    block_idx,
                    layout.width,
                    layout.height,
                    lines.len()
                );
            }
            BlockKind::Image { image } => {
                let size = image
                    .as_ref()
                    .and_then(|img| img.width.zip(img.height));
                debug_log!(
                    "layout block: idx={} kind=image width={} height={} image_size={:?}",
                    block_idx,
                    layout.width,
                    layout.height,
                    size
                );
            }
            BlockKind::MediaCard { lines, media_kind, .. } => {
                debug_log!(
                    "layout block: idx={} kind=media width={} height={} lines={} media_kind={:?}",
                    block_idx,
                    layout.width,
                    layout.height,
                    lines.len(),
                    media_kind
                );
            }
            BlockKind::FileCard { name_lines, .. } => {
                debug_log!(
                    "layout block: idx={} kind=file width={} height={} name_lines={}",
                    block_idx,
                    layout.width,
                    layout.height,
                    name_lines.len()
                );
            }
        }

        let layout_height = layout.height;
        let mut next_bottom = layout
            .y
            .saturating_add(layout_height)
            .saturating_add(padding);
        if !blocks.is_empty() {
            next_bottom = next_bottom.saturating_add(spacing_lg);
        }
        if next_bottom > config.max_height_px {
            let trunc_text_w = content_width
                .saturating_sub(bubble_pad_left + bubble_pad_right)
                .max(1);
            let trunc_lines = wrap_inline_text(
                "... truncated ...",
                trunc_text_w,
                font_size,
                face_size,
                font_weight_body,
                &mut text_measurer,
            );
            let mut max_line_w = 0u32;
            for line in &trunc_lines {
                max_line_w = max_line_w.max(line.width);
            }
            let bubble_w = (max_line_w + bubble_pad_left + bubble_pad_right)
                .min(content_width)
                .max(1);
            let trunc_height = bubble_pad_top + bubble_pad_bottom + line_height;
            let trunc_bottom = cursor_y.saturating_add(trunc_height).saturating_add(padding);
            if trunc_bottom <= config.max_height_px {
                debug_log!("layout truncate: adding truncation block");
                blocks.push(BlockLayout {
                    x: padding,
                    y: cursor_y,
                    width: bubble_w,
                    height: trunc_height,
                    kind: BlockKind::Text {
                        lines: trunc_lines,
                    },
                });
                cursor_y = cursor_y.saturating_add(trunc_height).saturating_add(spacing_lg);
            }
            break;
        }

        blocks.push(layout);
        cursor_y = cursor_y
            .saturating_add(layout_height)
            .saturating_add(spacing_lg);
    }

    if !blocks.is_empty() {
        cursor_y = cursor_y.saturating_sub(spacing_lg);
    }

    let canvas_bottom = if blocks.is_empty() {
        header_y + header_height + spacing_xxl
    } else {
        cursor_y
    };
    let canvas_height = canvas_bottom.saturating_add(padding);
    let background_height = canvas_height
        .max(config.canvas_width_px)
        .min(config.max_height_px)
        .max(1);

    let output_width = config.canvas_width_px.saturating_mul(scale);
    let output_height = background_height.saturating_mul(scale);
    debug_log!(
        "render canvas: background_height={} output={}x{}",
        background_height,
        output_width,
        output_height
    );

    let mut surface = skia_safe::surfaces::raster_n32_premul((
        output_width as i32,
        output_height as i32,
    ))
    .ok_or_else(|| "surface alloc failed".to_string())?;
    let canvas = surface.canvas();
    canvas.scale((scale as f32, scale as f32));

    let color_bg = color_from_hex(0xF2F2F2);
    let color_white = Color4f::new(1.0, 1.0, 1.0, 1.0);
    let color_border = color_from_hex(0xE0E0E0);
    let color_text = Color4f::new(0.0, 0.0, 0.0, 1.0);
    let color_meta = color_from_hex(0x666666);
    let color_muted = color_from_hex(0x888888);

    let mut bg_paint = Paint::default();
    bg_paint.set_color4f(color_bg, None);
    bg_paint.set_anti_alias(true);
    canvas.draw_rect(
        Rect::from_xywh(0.0, 0.0, config.canvas_width_px as f32, background_height as f32),
        &bg_paint,
    );

    let avatar_rect = Rect::from_xywh(
        header_x as f32,
        header_y as f32,
        avatar_size as f32,
        avatar_size as f32,
    );
    let avatar_rr = RRect::new_rect_xy(
        avatar_rect,
        avatar_size as f32 / 2.0,
        avatar_size as f32 / 2.0,
    );
    draw_shadowed_rrect(canvas, avatar_rr, 4.0, 0.30);
    let mut avatar_bg = Paint::default();
    avatar_bg.set_color4f(color_white, None);
    avatar_bg.set_anti_alias(true);
    canvas.draw_rrect(avatar_rr, &avatar_bg);
    let mut avatar_border = Paint::default();
    avatar_border.set_color4f(color_border, None);
    avatar_border.set_style(skia_safe::paint::Style::Stroke);
    avatar_border.set_stroke_width(1.0);
    avatar_border.set_anti_alias(true);
    canvas.draw_rrect(avatar_rr, &avatar_border);

    if let Some(avatar) = image_sources.avatar.as_ref().filter(|img| img.has_bytes()) {
        if let Some(image) = decode_image(avatar) {
            draw_image_cover_rounded(canvas, &image, avatar_rect, avatar_size as f32 / 2.0);
        }
    }

    let mut emoji_cache = EmojiRenderCache::new();
    draw_text_line(
        canvas,
        &font_collection,
        &mut emoji_cache,
        &title_text,
        header_text_x as f32,
        title_y as f32,
        title_size,
        font_weight_title,
        color_text,
    );
    draw_text_line(
        canvas,
        &font_collection,
        &mut emoji_cache,
        &meta_text,
        header_text_x as f32,
        meta_y as f32,
        meta_size,
        font_weight_body,
        color_meta,
    );

    let mut divider = Paint::default();
    divider.set_color4f(color_border, None);
    divider.set_anti_alias(true);
    divider.set_style(skia_safe::paint::Style::Stroke);
    divider.set_stroke_width(1.0);
    canvas.draw_line(
        (padding as f32, (header_y + header_height + spacing_xxl / 2) as f32),
        ((padding + content_width) as f32, (header_y + header_height + spacing_xxl / 2) as f32),
        &divider,
    );

    let mut face_cache: HashMap<String, ResolvedImage> = HashMap::new();
    let mut face_image_cache: HashMap<String, Option<Image>> = HashMap::new();

    for block in blocks {
        match block.kind {
            BlockKind::Text { lines } => {
                let rect = Rect::from_xywh(
                    block.x as f32,
                    block.y as f32,
                    block.width as f32,
                    block.height as f32,
                );
                let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                draw_shadowed_rrect(canvas, rr, 2.0, 0.10);

                let mut bubble_bg = Paint::default();
                bubble_bg.set_color4f(color_white, None);
                bubble_bg.set_anti_alias(true);
                canvas.draw_rrect(rr, &bubble_bg);
                let mut bubble_border = Paint::default();
                bubble_border.set_color4f(color_border, None);
                bubble_border.set_style(skia_safe::paint::Style::Stroke);
                bubble_border.set_stroke_width(1.0);
                bubble_border.set_anti_alias(true);
                canvas.draw_rrect(rr, &bubble_border);

                let line_x = block.x + bubble_pad_left;
                let line_y = block.y + bubble_pad_top + font_size;
                for (idx, line) in lines.iter().enumerate() {
                    let baseline_y =
                        line_y + line_height.saturating_mul(idx as u32);
                    let mut cursor_x = line_x;
                    for run in &line.runs {
                        match run {
                            InlineRun::Text(text) => {
                                if !text.is_empty() {
                                    draw_text_line(
                                        canvas,
                                        &font_collection,
                                        &mut emoji_cache,
                                        text,
                                        cursor_x as f32,
                                        baseline_y as f32,
                                        font_size,
                                        font_weight_body,
                                        color_text,
                                    );
                                    cursor_x = cursor_x.saturating_add(
                                        text_measurer
                                            .measure_text_width(text, font_size, font_weight_body),
                                    );
                                }
                            }
                            InlineRun::Face { id } => {
                                if let Some(face) = resolve_face_image(id, &mut face_cache) {
                                    let line_top = baseline_y.saturating_sub(font_size);
                                    let face_y = line_top
                                        .saturating_add(line_height.saturating_sub(face_size) / 2);
                                    let face_x = cursor_x;
                                    let face_image = face_image_cache
                                        .entry(id.clone())
                                        .or_insert_with(|| decode_image(&face));
                                    if let Some(image) = face_image.as_ref() {
                                        draw_image_cover_rounded(
                                            canvas,
                                            image,
                                            Rect::from_xywh(
                                                face_x as f32,
                                                face_y as f32,
                                                face_size as f32,
                                                face_size as f32,
                                            ),
                                            3.0,
                                        );
                                    } else {
                                        let fallback = format!("[face:{}]", id);
                                        draw_text_line(
                                            canvas,
                                            &font_collection,
                                            &mut emoji_cache,
                                            &fallback,
                                            cursor_x as f32,
                                            baseline_y as f32,
                                            font_size,
                                            font_weight_body,
                                            color_text,
                                        );
                                    }
                                    cursor_x = cursor_x.saturating_add(face_size);
                                } else {
                                    let fallback = format!("[face:{}]", id);
                                    draw_text_line(
                                        canvas,
                                        &font_collection,
                                        &mut emoji_cache,
                                        &fallback,
                                        cursor_x as f32,
                                        baseline_y as f32,
                                        font_size,
                                        font_weight_body,
                                        color_text,
                                    );
                                    cursor_x = cursor_x.saturating_add(
                                        text_measurer
                                            .measure_text_width(&fallback, font_size, font_weight_body),
                                    );
                                }
                            }
                        }
                    }
                }
            }
            BlockKind::Image { image } => {
                let rect = Rect::from_xywh(
                    block.x as f32,
                    block.y as f32,
                    block.width as f32,
                    block.height as f32,
                );
                let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                draw_shadowed_rrect(canvas, rr, 3.0, 0.20);

                let mut img_bg = Paint::default();
                img_bg.set_color4f(color_white, None);
                img_bg.set_anti_alias(true);
                canvas.draw_rrect(rr, &img_bg);
                let mut img_border = Paint::default();
                img_border.set_color4f(color_border, None);
                img_border.set_style(skia_safe::paint::Style::Stroke);
                img_border.set_stroke_width(1.0);
                img_border.set_anti_alias(true);
                canvas.draw_rrect(rr, &img_border);

                if let Some(img) = image.as_ref().filter(|img| img.has_bytes()) {
                    if let Some(decoded) = decode_image(img) {
                        draw_image_cover_rounded(canvas, &decoded, rect, radius_lg as f32);
                    }
                } else {
                    let text_x = block.x + bubble_pad_left;
                    let text_y = block.y + bubble_pad_top + font_size;
                    draw_text_line(
                        canvas,
                        &font_collection,
                        &mut emoji_cache,
                        "Image",
                        text_x as f32,
                        text_y as f32,
                        meta_size,
                        font_weight_body,
                        color_muted,
                    );
                }
            }
            BlockKind::MediaCard {
                lines,
                icon_text,
                media_kind,
            } => {
                let rect = Rect::from_xywh(
                    block.x as f32,
                    block.y as f32,
                    block.width as f32,
                    block.height as f32,
                );
                let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                draw_shadowed_rrect(canvas, rr, 2.0, 0.10);

                let mut card_bg = Paint::default();
                card_bg.set_color4f(color_white, None);
                card_bg.set_anti_alias(true);
                canvas.draw_rrect(rr, &card_bg);
                let mut card_border = Paint::default();
                card_border.set_color4f(color_border, None);
                card_border.set_style(skia_safe::paint::Style::Stroke);
                card_border.set_stroke_width(1.0);
                card_border.set_anti_alias(true);
                canvas.draw_rrect(rr, &card_border);

                let icon_x = block.x + card_padding;
                let icon_y = block.y + (block.height - card_icon_size) / 2;
                if matches!(media_kind, oqqwall_rust_core::MediaKind::Video) {
                    let cx = icon_x + card_icon_size / 2;
                    let cy = icon_y + card_icon_size / 2;
                    let mut icon_bg = Paint::default();
                    icon_bg.set_color4f(color_bg, None);
                    icon_bg.set_anti_alias(true);
                    canvas.draw_circle((cx as f32, cy as f32), (card_icon_size / 2) as f32, &icon_bg);
                    let mut icon_border = Paint::default();
                    icon_border.set_color4f(color_border, None);
                    icon_border.set_style(skia_safe::paint::Style::Stroke);
                    icon_border.set_stroke_width(1.0);
                    icon_border.set_anti_alias(true);
                    canvas.draw_circle((cx as f32, cy as f32), (card_icon_size / 2) as f32, &icon_border);

                    let tri_x = icon_x + card_icon_size / 3;
                    let tri_y = icon_y + card_icon_size / 4;
                    let tri_w = card_icon_size / 2;
                    let tri_h = card_icon_size / 2;
                    let mut tri = PathBuilder::new();
                    tri.move_to((tri_x as f32, tri_y as f32));
                    tri.line_to((tri_x as f32, (tri_y + tri_h) as f32));
                    tri.line_to(((tri_x + tri_w) as f32, (tri_y + tri_h / 2) as f32));
                    tri.close();
                    let tri = tri.detach();
                    let mut tri_paint = Paint::default();
                    tri_paint.set_color4f(color_from_hex(0x007AFF), None);
                    tri_paint.set_anti_alias(true);
                    canvas.draw_path(&tri, &tri_paint);
                } else {
                    let icon_rect = Rect::from_xywh(
                        icon_x as f32,
                        icon_y as f32,
                        card_icon_size as f32,
                        card_icon_size as f32,
                    );
                    let icon_rr = RRect::new_rect_xy(icon_rect, 4.0, 4.0);
                    let mut icon_bg = Paint::default();
                    icon_bg.set_color4f(color_bg, None);
                    icon_bg.set_anti_alias(true);
                    canvas.draw_rrect(icon_rr, &icon_bg);
                    let mut icon_border = Paint::default();
                    icon_border.set_color4f(color_border, None);
                    icon_border.set_style(skia_safe::paint::Style::Stroke);
                    icon_border.set_stroke_width(1.0);
                    icon_border.set_anti_alias(true);
                    canvas.draw_rrect(icon_rr, &icon_border);

                    let icon_center_x = icon_x + card_icon_size / 2;
                    let icon_center_y = icon_y + card_icon_size / 2;
                    if let Some((paragraph, metrics)) = build_line_paragraph(
                        &font_collection,
                        &icon_text,
                        11,
                        font_weight_body,
                        color_meta,
                    ) {
                        let icon_baseline =
                            center_baseline(icon_center_y as f32, &metrics);
                        let icon_x = icon_center_x as f32 - metrics.width * 0.5;
                        let top_y = icon_baseline - metrics.baseline;
                        paragraph.paint(canvas, (icon_x, top_y));
                    }
                }

                let text_x = block.x + card_padding + card_icon_size + card_icon_gap;
                let text_y = block.y + card_padding + font_size;
                for (idx, line) in lines.iter().enumerate() {
                    let baseline = text_y + card_line_height.saturating_mul(idx as u32);
                    draw_text_line(
                        canvas,
                        &font_collection,
                        &mut emoji_cache,
                        line,
                        text_x as f32,
                        baseline as f32,
                        font_size,
                        font_weight_body,
                        color_text,
                    );
                }
            }
            BlockKind::FileCard {
                name_lines,
                meta_line,
                icon_text,
            } => {
                let rect = Rect::from_xywh(
                    block.x as f32,
                    block.y as f32,
                    block.width as f32,
                    block.height as f32,
                );
                let rr = RRect::new_rect_xy(rect, radius_lg as f32, radius_lg as f32);
                draw_shadowed_rrect(canvas, rr, 2.0, 0.10);

                let mut card_bg = Paint::default();
                card_bg.set_color4f(color_white, None);
                card_bg.set_anti_alias(true);
                canvas.draw_rrect(rr, &card_bg);
                let mut card_border = Paint::default();
                card_border.set_color4f(color_border, None);
                card_border.set_style(skia_safe::paint::Style::Stroke);
                card_border.set_stroke_width(1.0);
                card_border.set_anti_alias(true);
                canvas.draw_rrect(rr, &card_border);

                let icon_x = block.x + block.width.saturating_sub(file_padding + file_icon_size);
                let icon_y = block.y + (block.height - file_icon_size) / 2;
                let icon_rect = Rect::from_xywh(
                    icon_x as f32,
                    icon_y as f32,
                    file_icon_size as f32,
                    file_icon_size as f32,
                );
                let icon_rr = RRect::new_rect_xy(icon_rect, 4.0, 4.0);
                let mut icon_bg = Paint::default();
                icon_bg.set_color4f(color_bg, None);
                icon_bg.set_anti_alias(true);
                canvas.draw_rrect(icon_rr, &icon_bg);
                let mut icon_border = Paint::default();
                icon_border.set_color4f(color_border, None);
                icon_border.set_style(skia_safe::paint::Style::Stroke);
                icon_border.set_stroke_width(1.0);
                icon_border.set_anti_alias(true);
                canvas.draw_rrect(icon_rr, &icon_border);

                let icon_center_x = icon_x + file_icon_size / 2;
                let icon_center_y = icon_y + file_icon_size / 2;
                if let Some((paragraph, metrics)) = build_line_paragraph(
                    &font_collection,
                    &icon_text,
                    11,
                    font_weight_body,
                    color_meta,
                ) {
                    let icon_baseline = center_baseline(icon_center_y as f32, &metrics);
                    let icon_x = icon_center_x as f32 - metrics.width * 0.5;
                    let top_y = icon_baseline - metrics.baseline;
                    paragraph.paint(canvas, (icon_x, top_y));
                }

                let text_x = block.x + file_padding;
                let name_height = name_lines.len() as u32 * file_line_height;
                let meta_height = if meta_line.is_some() {
                    file_meta_height + file_meta_gap
                } else {
                    0
                };
                let text_height = name_height + meta_height;
                let content_height = file_icon_size.max(text_height);
                let text_y =
                    block.y + file_padding + (content_height - text_height) / 2 + font_size;

                for (idx, line) in name_lines.iter().enumerate() {
                    let baseline = text_y + file_line_height.saturating_mul(idx as u32);
                    draw_text_line(
                        canvas,
                        &font_collection,
                        &mut emoji_cache,
                        line,
                        text_x as f32,
                        baseline as f32,
                        font_size,
                        font_weight_body,
                        color_text,
                    );
                }
                if let Some(meta) = meta_line {
                    let meta_y = text_y + name_height + file_meta_gap + file_meta_height;
                    draw_text_line(
                        canvas,
                        &font_collection,
                        &mut emoji_cache,
                        &meta,
                        text_x as f32,
                        meta_y as f32,
                        11,
                        font_weight_body,
                        color_muted,
                    );
                }
            }
        }
    }

    let image = surface.image_snapshot();
    let data = image
        .encode(None, EncodedImageFormat::PNG, None)
        .ok_or_else(|| "encode png failed".to_string())?;
    Ok(data.as_bytes().to_vec())
}

fn color_from_hex(hex: u32) -> Color4f {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;
    Color4f::new(r, g, b, 1.0)
}

struct LineMetricsSnapshot {
    baseline: f32,
    ascent: f32,
    descent: f32,
    width: f32,
}

#[derive(Debug, Clone, Copy)]
struct EmojiGlyphMeta {
    width: u8,
    height: u8,
    bearing_x: i8,
    bearing_y: i8,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmojiGlyphRecord {
    glyph_id: u16,
    width: u8,
    height: u8,
    bearing_x: i8,
    bearing_y: i8,
}

#[derive(Debug, Serialize, Deserialize)]
struct EmojiPngMetadata {
    strike_ppem: u8,
    glyphs: Vec<EmojiGlyphRecord>,
}

#[derive(Debug)]
struct EmojiPngStore {
    res_prefix: &'static str,
    strike_ppem: u8,
    glyphs: HashMap<u16, EmojiGlyphMeta>,
    png_cache: HashMap<u16, &'static [u8]>,
}

struct EmojiRenderCache {
    store: Option<&'static Mutex<EmojiPngStore>>,
    image_cache: HashMap<u16, Option<Image>>,
}

impl EmojiRenderCache {
    fn new() -> Self {
        Self {
            store: emoji_png_store(),
            image_cache: HashMap::new(),
        }
    }

    fn draw_over_paragraph(
        &mut self,
        canvas: &Canvas,
        paragraph: &mut Paragraph,
        origin_x: f32,
        origin_y: f32,
    ) {
        let Some(store) = self.store else { return; };
        let mut store = match store.lock() {
            Ok(guard) => guard,
            Err(guard) => guard.into_inner(),
        };
        if store.strike_ppem == 0 {
            return;
        }
        let strike_ppem = store.strike_ppem as f32;
        let image_cache = &mut self.image_cache;
        paragraph.visit(|_, info| {
            let Some(info) = info else { return; };
            let font = info.font();
            if font.size() <= 0.0 {
                return;
            }
            let typeface = font.typeface();
            if !is_emoji_typeface(&typeface) {
                return;
            }
            let scale = font.size() / strike_ppem;
            let origin = info.origin();
            let positions = info.positions();
            let glyphs = info.glyphs();
            let sampling = SamplingOptions {
                filter: skia_safe::FilterMode::Linear,
                mipmap: skia_safe::MipmapMode::None,
                ..SamplingOptions::default()
            };
            let paint = Paint::default();
            for (idx, glyph_id) in glyphs.iter().enumerate() {
                let glyph_id = *glyph_id as u16;
                let meta = match store.glyphs.get(&glyph_id).copied() {
                    Some(meta) => meta,
                    None => continue,
                };
                let png_bytes = match store.png_cache.get(&glyph_id).copied() {
                    Some(bytes) => bytes,
                    None => {
                        let path = emoji_png_resource_path(store.res_prefix, glyph_id);
                        let bytes = match embedded_resource_bytes(&path) {
                            Some(bytes) => bytes,
                            None => continue,
                        };
                        store.png_cache.insert(glyph_id, bytes);
                        bytes
                    }
                };
                let image = image_cache
                    .entry(glyph_id)
                    .or_insert_with(|| decode_emoji_image(png_bytes));
                let Some(image) = image.as_ref() else { continue; };
                let pos = positions.get(idx).copied().unwrap_or_default();
                let width = meta.width as f32 * scale;
                let height = meta.height as f32 * scale;
                if width <= 0.0 || height <= 0.0 {
                    continue;
                }
                let x = origin_x + origin.x + pos.x + meta.bearing_x as f32 * scale;
                let y = origin_y + origin.y + pos.y - meta.bearing_y as f32 * scale;
                let dst = Rect::from_xywh(x, y, width, height);
                canvas.draw_image_rect_with_sampling_options(image, None, dst, sampling, &paint);
            }
        });
    }
}

fn decode_emoji_image(bytes: &[u8]) -> Option<Image> {
    let data = Data::new_copy(bytes);
    Image::from_encoded(data)
}

fn is_emoji_typeface(typeface: &Typeface) -> bool {
    match typeface.family_name().as_str() {
        "Apple Color Emoji" | "OQQWall Emoji" => true,
        _ => false,
    }
}

fn build_text_style(font_size: u32, font_weight: u32, color: Color4f) -> TextStyle {
    debug_log!(
        "text style: size={} weight={} color={:?} families={:?}",
        font_size,
        font_weight,
        color,
        FONT_FAMILIES
    );
    let mut ts = TextStyle::new();
    ts.set_font_size(font_size as f32);
    ts.set_font_families(&FONT_FAMILIES);
    let font_style = FontStyle::new(
        Weight::from(font_weight as i32),
        Width::NORMAL,
        Slant::Upright,
    );
    ts.set_font_style(font_style);
    let mut paint = Paint::default();
    paint.set_color4f(color, None);
    ts.set_foreground_paint(&paint);
    ts
}

fn build_emoji_text_style(font_size: u32, font_weight: u32, color: Color4f) -> TextStyle {
    let mut ts = TextStyle::new();
    ts.set_font_size(font_size as f32);
    let mut paint = Paint::default();
    paint.set_color4f(color, None);
    ts.set_foreground_paint(&paint);
    if let Some(tf) = emoji_typeface() {
        debug_log!(
            "emoji style: size={} weight={} typeface_family={} postscript={:?}",
            font_size,
            font_weight,
            tf.family_name(),
            tf.post_script_name()
        );
        ts.set_font_style(tf.font_style());
        ts.set_typeface(Some(tf));
    } else {
        debug_log!(
            "emoji style: size={} weight={} families={:?}",
            font_size,
            font_weight,
            EMOJI_FONT_FAMILIES
        );
        ts.set_font_families(&EMOJI_FONT_FAMILIES);
        let font_style = FontStyle::new(
            Weight::from(font_weight as i32),
            Width::NORMAL,
            Slant::Upright,
        );
        ts.set_font_style(font_style);
    }
    ts
}

fn emoji_typeface() -> Option<Typeface> {
    static EMOJI_TF: OnceLock<Option<Typeface>> = OnceLock::new();
    EMOJI_TF
        .get_or_init(|| {
            let bytes = match embedded_resource_bytes("fonts/AppleColorEmoji.ttf") {
                Some(bytes) => bytes,
                None => {
                    debug_log!("emoji typeface: missing embedded AppleColorEmoji.ttf");
                    return None;
                }
            };
            let mgr = skia_safe::FontMgr::new();
            let tf = mgr.new_from_data(bytes, 0usize);
            if let Some(ref tf) = tf {
                debug_log!(
                    "emoji typeface loaded: family={} postscript={:?} bytes={}",
                    tf.family_name(),
                    tf.post_script_name(),
                    bytes.len()
                );
            } else {
                debug_log!("emoji typeface load failed");
            }
            tf
        })
        .clone()
}

const EMOJI_PNG_RES_PREFIX: &str = "emoji_png/apple_color_emoji";
const EMOJI_PNG_METADATA_PATH: &str = "emoji_png/apple_color_emoji/metadata.json";

fn emoji_png_store() -> Option<&'static Mutex<EmojiPngStore>> {
    static EMOJI_PNG_STORE: OnceLock<Option<Mutex<EmojiPngStore>>> = OnceLock::new();
    EMOJI_PNG_STORE
        .get_or_init(init_emoji_png_store_inner)
        .as_ref()
}

fn init_emoji_png_store_inner() -> Option<Mutex<EmojiPngStore>> {
    let metadata = match load_embedded_emoji_png_metadata() {
        Some(metadata) => metadata,
        None => {
            debug_log!(
                "emoji png: metadata missing; run scripts/extract_apple_emoji_pngs.py"
            );
            return None;
        }
    };
    let glyphs = metadata
        .glyphs
        .into_iter()
        .map(|record| {
            (
                record.glyph_id,
                EmojiGlyphMeta {
                    width: record.width,
                    height: record.height,
                    bearing_x: record.bearing_x,
                    bearing_y: record.bearing_y,
                },
            )
        })
        .collect();
    Some(Mutex::new(EmojiPngStore {
        res_prefix: EMOJI_PNG_RES_PREFIX,
        strike_ppem: metadata.strike_ppem,
        glyphs,
        png_cache: HashMap::new(),
    }))
}

fn emoji_png_resource_path(prefix: &str, glyph_id: u16) -> String {
    format!("{}/gid_{:04x}.png", prefix, glyph_id)
}

fn load_embedded_emoji_png_metadata() -> Option<EmojiPngMetadata> {
    let bytes = embedded_resource_bytes(EMOJI_PNG_METADATA_PATH)?;
    serde_json::from_slice(bytes).ok()
}

fn append_text_with_emoji_runs(
    builder: &mut ParagraphBuilder,
    text: &str,
    base_style: &TextStyle,
    emoji_style: &TextStyle,
) {
    if !contains_emoji(text) {
        builder.add_text(text);
        return;
    }
    for (run, is_emoji) in split_emoji_runs(text) {
        if run.is_empty() {
            continue;
        }
        if is_emoji {
            builder.push_style(emoji_style);
        } else {
            builder.push_style(base_style);
        }
        builder.add_text(&run);
        builder.pop();
    }
}

fn contains_emoji(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let next = chars.peek().copied();
        if is_emoji_char_with_next(ch, next) {
            return true;
        }
    }
    false
}

fn split_emoji_runs(text: &str) -> Vec<(String, bool)> {
    let mut runs = Vec::new();
    let mut current = String::new();
    let mut current_is_emoji: Option<bool> = None;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let next = chars.peek().copied();
        let is_emoji = is_emoji_char_with_next(ch, next);
        match current_is_emoji {
            Some(flag) if flag == is_emoji => {
                current.push(ch);
            }
            Some(flag) => {
                runs.push((std::mem::take(&mut current), flag));
                current.push(ch);
                current_is_emoji = Some(is_emoji);
            }
            None => {
                current.push(ch);
                current_is_emoji = Some(is_emoji);
            }
        }
    }
    if let Some(flag) = current_is_emoji {
        if !current.is_empty() {
            runs.push((current, flag));
        }
    }
    runs
}

fn is_emoji_char_with_next(ch: char, next: Option<char>) -> bool {
    if is_emoji_like(ch) {
        return true;
    }
    if matches!(ch, '#' | '*' | '0'..='9') {
        return matches!(next, Some('\u{FE0F}') | Some('\u{20E3}'));
    }
    false
}

fn is_emoji_like(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x00A9
            | 0x00AE
            | 0x2122
            | 0x2934
            | 0x2935
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0xFE0E
            | 0xFE0F
            | 0x200D
            | 0x20E3
    ) || (0x1F000..=0x1FAFF).contains(&code)
        || (0x1F1E6..=0x1F1FF).contains(&code)
        || (0x1F3FB..=0x1F3FF).contains(&code)
        || (0x2300..=0x23FF).contains(&code)
        || (0x2600..=0x27BF).contains(&code)
        || (0x2B00..=0x2BFF).contains(&code)
        || (0xE0020..=0xE007F).contains(&code)
}

fn build_line_paragraph(
    font_collection: &FontCollection,
    text: &str,
    font_size: u32,
    font_weight: u32,
    color: Color4f,
) -> Option<(Paragraph, LineMetricsSnapshot)> {
    if text.is_empty() {
        return None;
    }
    debug_log!(
        "paragraph build: text_len={} size={} weight={}",
        text.len(),
        font_size,
        font_weight
    );
    let mut ps = ParagraphStyle::new();
    let ts = build_text_style(font_size, font_weight, color);
    let emoji_ts = build_emoji_text_style(font_size, font_weight, color);
    ps.set_text_style(&ts);
    let mut builder = ParagraphBuilder::new(&ps, font_collection.clone());
    append_text_with_emoji_runs(&mut builder, text, &ts, &emoji_ts);
    let mut paragraph = builder.build();
    paragraph.layout(MEASURE_MAX_WIDTH);
    let line_metrics = paragraph.get_line_metrics();
    let metrics = line_metrics.get(0)?;
    debug_log!(
        "paragraph metrics: text_len={} baseline={} ascent={} descent={} width={}",
        text.len(),
        metrics.baseline,
        metrics.ascent,
        metrics.descent,
        metrics.width
    );
    let snapshot = LineMetricsSnapshot {
        baseline: metrics.baseline as f32,
        ascent: metrics.ascent as f32,
        descent: metrics.descent as f32,
        width: metrics.width as f32,
    };
    Some((paragraph, snapshot))
}

fn draw_shadowed_rrect(canvas: &Canvas, rr: RRect, blur: f32, alpha: f32) {
    let shadow = image_filters::drop_shadow_only(
        (0.0, 0.0),
        (blur, blur),
        Color4f::new(0.0, 0.0, 0.0, alpha),
        None,
        None,
        image_filters::CropRect::from(None),
    );
    if let Some(filter) = shadow {
        let mut paint = Paint::default();
        paint.set_image_filter(filter);
        paint.set_anti_alias(true);
        canvas.draw_rrect(rr, &paint);
    }
}

fn draw_text_line(
    canvas: &Canvas,
    font_collection: &FontCollection,
    emoji_cache: &mut EmojiRenderCache,
    text: &str,
    x: f32,
    baseline_y: f32,
    font_size: u32,
    font_weight: u32,
    color: Color4f,
) {
    debug_log!(
        "text draw: text_len={} size={} weight={} x={} baseline_y={}",
        text.len(),
        font_size,
        font_weight,
        x,
        baseline_y
    );
    if let Some((mut paragraph, metrics)) =
        build_line_paragraph(font_collection, text, font_size, font_weight, color)
    {
        let top_y = baseline_y - metrics.baseline;
        paragraph.paint(canvas, (x, top_y));
        if contains_emoji(text) {
            emoji_cache.draw_over_paragraph(canvas, &mut paragraph, x, top_y);
        }
    }
}

fn center_baseline(center_y: f32, metrics: &LineMetricsSnapshot) -> f32 {
    center_y + (metrics.ascent - metrics.descent) * 0.5
}

fn decode_image(image: &ResolvedImage) -> Option<Image> {
    let bytes = image.bytes.as_ref()?;
    debug_log!("image decode: bytes={}", bytes.len());
    let data = Data::new_copy(bytes);
    Image::from_encoded(data)
}

fn draw_image_cover_rounded(canvas: &Canvas, img: &Image, dst: Rect, radius: f32) {
    let sw = img.width() as f32;
    let sh = img.height() as f32;
    debug_log!(
        "image draw: src=({}x{}) dst=({},{} {}x{}) radius={}",
        sw,
        sh,
        dst.x(),
        dst.y(),
        dst.width(),
        dst.height(),
        radius
    );
    if sw <= 0.0 || sh <= 0.0 || dst.width() <= 0.0 || dst.height() <= 0.0 {
        return;
    }
    let scale = (dst.width() / sw).max(dst.height() / sh);
    let crop_w = dst.width() / scale;
    let crop_h = dst.height() / scale;
    let crop_x = (sw - crop_w) * 0.5;
    let crop_y = (sh - crop_h) * 0.5;
    let src = Rect::from_xywh(crop_x.max(0.0), crop_y.max(0.0), crop_w.max(1.0), crop_h.max(1.0));
    let rr = RRect::new_rect_xy(dst, radius, radius);

    canvas.save();
    canvas.clip_rrect(rr, Some(ClipOp::Intersect), Some(true));

    let sampling = SamplingOptions {
        filter: skia_safe::FilterMode::Linear,
        mipmap: skia_safe::MipmapMode::None,
        ..SamplingOptions::default()
    };
    let paint = Paint::default();
    canvas.draw_image_rect_with_sampling_options(
        img,
        Some((&src, SrcRectConstraint::Fast)),
        &dst,
        sampling,
        &paint,
    );
    canvas.restore();
}

async fn resolve_image_sources(
    state: &StateView,
    draft: &Draft,
    header: &HeaderInfo,
) -> RenderImageSources {
    let mut client = None;
    let mut block_images = vec![None; draft.blocks.len()];
    let mut block_labels = vec![None; draft.blocks.len()];
    for (idx, block) in draft.blocks.iter().enumerate() {
        if let DraftBlock::Attachment { kind, reference } = block {
            if *kind == oqqwall_rust_core::MediaKind::Image {
                block_images[idx] =
                    resolve_media_reference_for_image(reference, state, &mut client).await;
            } else {
                block_labels[idx] = resolve_media_reference_for_label(reference, state);
            }
        }
    }
    let avatar = if header.user_id == "unknown" {
        None
    } else {
        let url = format!(
            "https://qlogo2.store.qq.com/qzone/{0}/{0}/640",
            header.user_id
        );
        resolve_url_to_image(&url, &mut client).await
    };
    RenderImageSources {
        avatar,
        block_images,
        block_labels,
    }
}

async fn resolve_media_reference_for_image(
    reference: &oqqwall_rust_core::MediaReference,
    state: &StateView,
    client: &mut Option<Client>,
) -> Option<ResolvedImage> {
    match reference {
        oqqwall_rust_core::MediaReference::Blob { blob_id } => {
            debug_log!("media image: blob_id={:?}", blob_id);
            resolve_blob_image(state, *blob_id)
        }
        oqqwall_rust_core::MediaReference::RemoteUrl { url } => {
            debug_log!("media image: remote url={}", url);
            resolve_url_to_image(url, client).await
        }
    }
}

fn resolve_media_reference_for_label(
    reference: &oqqwall_rust_core::MediaReference,
    _state: &StateView,
) -> Option<String> {
    match reference {
        oqqwall_rust_core::MediaReference::Blob { .. } => None,
        oqqwall_rust_core::MediaReference::RemoteUrl { url } => Some(url.clone()),
    }
}

fn resolve_face_image(id: &str, cache: &mut HashMap<String, ResolvedImage>) -> Option<ResolvedImage> {
    if let Some(found) = cache.get(id) {
        debug_log!("face cache hit: id={}", id);
        return Some(found.clone());
    }
    let path = Path::new("res").join("face").join(format!("{}.png", id));
    debug_log!("face load: id={} path={}", id, path.display());
    let path_str = path.to_string_lossy();
    let resolved = resolved_image_from_path(&path_str)?;
    cache.insert(id.to_string(), resolved.clone());
    Some(resolved)
}

fn resolve_blob_image(state: &StateView, blob_id: oqqwall_rust_core::BlobId) -> Option<ResolvedImage> {
    let path = state
        .blobs
        .get(&blob_id)
        .and_then(|meta| meta.persisted_path.clone())?;
    debug_log!("blob image path: blob_id={:?} path={}", blob_id, path);
    resolved_image_from_path(&path)
}

async fn resolve_url_to_image(url: &str, client: &mut Option<Client>) -> Option<ResolvedImage> {
    if url.starts_with("data:") {
        debug_log!("image load data url");
        return resolved_image_from_data_url(url);
    }
    if let Some(path) = url.strip_prefix("file://") {
        debug_log!("image load file url: {}", path);
        return resolved_image_from_path(path);
    }
    if Path::new(url).exists() {
        debug_log!("image load local path: {}", url);
        return resolved_image_from_path(url);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        debug_log!("image load remote: {}", url);
        if client.is_none() {
            let built = Client::builder()
                .timeout(Duration::from_secs(6))
                .build()
                .ok()?;
            *client = Some(built);
        }
        let client = client.as_ref()?;
        if let Some((bytes, _content_type)) = fetch_remote_bytes(client, url).await {
            return Some(ResolvedImage::from_bytes(bytes));
        }
    }
    None
}

fn resolved_image_from_path(path: &str) -> Option<ResolvedImage> {
    let path_obj = Path::new(path);
    if let Some(bytes) = embedded_bytes_for_path(path_obj) {
        debug_log!("image load embedded: {}", path);
        return Some(ResolvedImage::from_bytes(bytes.to_vec()));
    }
    debug_log!("image load disk: {}", path);
    let bytes = fs::read(path).ok()?;
    Some(ResolvedImage::from_bytes(bytes))
}

fn resolved_image_from_data_url(url: &str) -> Option<ResolvedImage> {
    let (_mime, bytes) = parse_data_url(url)?;
    Some(ResolvedImage::from_bytes(bytes))
}

fn image_size_from_bytes(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = imagesize::blob_size(bytes).ok()?;
    let width = u32::try_from(size.width).ok()?;
    let height = u32::try_from(size.height).ok()?;
    Some((width, height))
}

fn parse_data_url(source: &str) -> Option<(Option<String>, Vec<u8>)> {
    let payload = source.strip_prefix("data:")?;
    let (meta, data) = payload.split_once(',')?;
    let mime = meta.split(';').next().map(|value| value.to_string());
    let bytes = if meta.contains(";base64") {
        STANDARD.decode(data).ok()?
    } else {
        data.as_bytes().to_vec()
    };
    Some((mime, bytes))
}

async fn fetch_remote_bytes(client: &Client, url: &str) -> Option<(Vec<u8>, Option<String>)> {
    debug_log!("http fetch: {}", url);
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        debug_log!("http fetch failed: {}", url);
        return None;
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let bytes = response.bytes().await.ok()?.to_vec();
    Some((bytes, content_type))
}

fn extract_filename(url: &str) -> Option<String> {
    let trimmed = url.split('?').next().unwrap_or(url);
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn file_icon_text(name: &str) -> String {
    let ext = name
        .rsplit('.')
        .next()
        .filter(|part| *part != name)
        .unwrap_or("");
    if ext.is_empty() {
        return "FILE".to_string();
    }
    let mut out = ext.to_ascii_uppercase();
    if out.len() > 4 {
        out.truncate(4);
    }
    out
}

fn media_label(kind: oqqwall_rust_core::MediaKind) -> &'static str {
    match kind {
        oqqwall_rust_core::MediaKind::Image => "Image",
        oqqwall_rust_core::MediaKind::Video => "Video",
        oqqwall_rust_core::MediaKind::File => "File",
        oqqwall_rust_core::MediaKind::Audio => "Audio",
        oqqwall_rust_core::MediaKind::Other => "Attachment",
    }
}

fn media_icon_text(kind: oqqwall_rust_core::MediaKind) -> String {
    match kind {
        oqqwall_rust_core::MediaKind::Video => "VID".to_string(),
        oqqwall_rust_core::MediaKind::Audio => "AUD".to_string(),
        oqqwall_rust_core::MediaKind::Other => "ATT".to_string(),
        _ => "FILE".to_string(),
    }
}

fn truncate_text(
    text: &str,
    max_width: u32,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        let width = measurer.measure_text_width(&out, font_size, font_weight);
        if width > max_width {
            out.pop();
            let ellipsis_width = measurer.measure_text_width("...", font_size, font_weight);
            let base_width = measurer.measure_text_width(&out, font_size, font_weight);
            if base_width + ellipsis_width <= max_width && !out.is_empty() {
                out.push_str("...");
            }
            return out;
        }
    }
    out
}

fn limit_lines(
    mut lines: Vec<String>,
    max_lines: usize,
    max_width: u32,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> Vec<String> {
    if lines.len() <= max_lines {
        return lines;
    }
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        let padded = format!("{}...", last);
        *last = truncate_text(&padded, max_width, font_size, font_weight, measurer);
    }
    lines
}

fn wrap_inline_text(
    text: &str,
    max_width: u32,
    font_size: u32,
    face_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> Vec<InlineLine> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(InlineLine {
                runs: Vec::new(),
                width: 0,
            });
            continue;
        }
        let atoms = parse_inline_atoms(raw_line);
        let mut current: Vec<InlineAtom> = Vec::new();
        let mut segment_text = String::new();
        let mut base_width = 0u32;
        let mut last_break: Option<usize> = None;
        for atom in atoms {
            let is_break = inline_atom_is_break(&atom);
            match &atom {
                InlineAtom::Char(ch) => segment_text.push(*ch),
                InlineAtom::Face(_) => {
                    if !segment_text.is_empty() {
                        base_width = base_width.saturating_add(measurer.measure_text_width(
                            &segment_text,
                            font_size,
                            font_weight,
                        ));
                        segment_text.clear();
                    }
                    base_width = base_width.saturating_add(face_size);
                }
            }
            current.push(atom);
            let segment_width =
                measurer.measure_text_width(&segment_text, font_size, font_weight);
            let current_width = base_width.saturating_add(segment_width);
            if current_width > max_width && current.len() > 1 {
                if let Some(break_idx) = last_break {
                    let mut line_atoms = current[..break_idx].to_vec();
                    trim_inline_trailing_spaces(&mut line_atoms);
                    lines.push(build_inline_line(
                        &line_atoms,
                        font_size,
                        face_size,
                        font_weight,
                        measurer,
                    ));
                    let mut remainder = current[break_idx..].to_vec();
                    trim_inline_leading_spaces(&mut remainder);
                    current = remainder;
                } else {
                    let last_atom = current.pop().unwrap();
                    let line_atoms = current;
                    lines.push(build_inline_line(
                        &line_atoms,
                        font_size,
                        face_size,
                        font_weight,
                        measurer,
                    ));
                    current = vec![last_atom];
                }
                let (next_segment_text, next_base_width) = rebuild_inline_measure_state(
                    &current,
                    font_size,
                    face_size,
                    font_weight,
                    measurer,
                );
                segment_text = next_segment_text;
                base_width = next_base_width;
                last_break = None;
                if let Some(last_atom) = current.last() {
                    if inline_atom_is_break(last_atom) {
                        last_break = Some(current.len());
                    }
                }
            }
            if is_break {
                last_break = Some(current.len());
            }
        }
        if !current.is_empty() {
            lines.push(build_inline_line(
                &current,
                font_size,
                face_size,
                font_weight,
                measurer,
            ));
        }
    }
    if lines.is_empty() {
        lines.push(InlineLine {
            runs: Vec::new(),
            width: 0,
        });
    }
    lines
}

fn rebuild_inline_measure_state(
    atoms: &[InlineAtom],
    font_size: u32,
    face_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> (String, u32) {
    let mut segment_text = String::new();
    let mut base_width = 0u32;
    for atom in atoms {
        match atom {
            InlineAtom::Char(ch) => segment_text.push(*ch),
            InlineAtom::Face(_) => {
                if !segment_text.is_empty() {
                    base_width = base_width.saturating_add(measurer.measure_text_width(
                        &segment_text,
                        font_size,
                        font_weight,
                    ));
                    segment_text.clear();
                }
                base_width = base_width.saturating_add(face_size);
            }
        }
    }
    (segment_text, base_width)
}

fn wrap_text(
    text: &str,
    max_width: u32,
    font_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current: Vec<char> = Vec::new();
        let mut current_text = String::new();
        let mut last_break: Option<usize> = None;
        for ch in raw_line.chars() {
            current.push(ch);
            current_text.push(ch);
            let current_width =
                measurer.measure_text_width(&current_text, font_size, font_weight);
            if current_width > max_width && current.len() > 1 {
                if let Some(break_idx) = last_break {
                    let line: String = current[..break_idx].iter().collect();
                    lines.push(line.trim_end().to_string());
                    let mut remainder: Vec<char> = current[break_idx..].iter().copied().collect();
                    while remainder.first().map(|c| c.is_whitespace()).unwrap_or(false) {
                        remainder.remove(0);
                    }
                    current = remainder;
                } else {
                    let last = current.pop().unwrap();
                    let line: String = current.iter().collect();
                    lines.push(line);
                    current.clear();
                    current.push(last);
                }
                current_text = current.iter().collect();
                last_break = None;
                if let Some(last_ch) = current.last() {
                    if is_break_char(*last_ch) {
                        last_break = Some(current.len());
                    }
                }
            }
            if is_break_char(ch) {
                last_break = Some(current.len());
            }
        }
        if !current.is_empty() {
            lines.push(current.iter().collect());
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn parse_inline_atoms(line: &str) -> Vec<InlineAtom> {
    let mut atoms = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'[' && bytes.get(idx + 1) == Some(&b'[') {
            let rest = &line[idx..];
            if rest.starts_with("[[face:") {
                let after_prefix = idx + "[[face:".len();
                if after_prefix <= line.len() {
                    if let Some(close) = line[after_prefix..].find("]]") {
                        let face_id = &line[after_prefix..after_prefix + close];
                        if !face_id.is_empty() && face_id.chars().all(|c| c.is_ascii_digit()) {
                            atoms.push(InlineAtom::Face(face_id.to_string()));
                            idx = after_prefix + close + 2;
                            continue;
                        }
                    }
                }
            }
        }
        let ch = line[idx..].chars().next().unwrap();
        atoms.push(InlineAtom::Char(ch));
        idx += ch.len_utf8();
    }
    atoms
}

fn inline_atom_is_break(atom: &InlineAtom) -> bool {
    match atom {
        InlineAtom::Char(ch) => is_break_char(*ch),
        InlineAtom::Face(_) => false,
    }
}

fn inline_atom_is_whitespace(atom: &InlineAtom) -> bool {
    matches!(atom, InlineAtom::Char(ch) if ch.is_whitespace())
}

fn trim_inline_leading_spaces(atoms: &mut Vec<InlineAtom>) {
    while atoms.first().map(inline_atom_is_whitespace).unwrap_or(false) {
        atoms.remove(0);
    }
}

fn trim_inline_trailing_spaces(atoms: &mut Vec<InlineAtom>) {
    while atoms.last().map(inline_atom_is_whitespace).unwrap_or(false) {
        atoms.pop();
    }
}

fn build_inline_line(
    atoms: &[InlineAtom],
    font_size: u32,
    face_size: u32,
    font_weight: u32,
    measurer: &mut TextMeasurer,
) -> InlineLine {
    let mut runs = Vec::new();
    let mut current = String::new();
    let mut width = 0u32;
    for atom in atoms {
        match atom {
            InlineAtom::Char(ch) => {
                current.push(*ch);
            }
            InlineAtom::Face(id) => {
                if !current.is_empty() {
                    width = width.saturating_add(
                        measurer.measure_text_width(&current, font_size, font_weight),
                    );
                    runs.push(InlineRun::Text(current.clone()));
                    current.clear();
                }
                runs.push(InlineRun::Face { id: id.clone() });
                width = width.saturating_add(face_size);
            }
        }
    }
    if !current.is_empty() {
        width =
            width.saturating_add(measurer.measure_text_width(&current, font_size, font_weight));
        runs.push(InlineRun::Text(current));
    }
    InlineLine { runs, width }
}

fn is_break_char(ch: char) -> bool {
    if ch.is_whitespace() {
        return true;
    }
    matches!(
        ch,
        '-' | '/' | '_' | '.' | ',' | ';' | ':' | '?' | '!' | '，' | '。' | '；' | '、' | '：'
    )
}


fn persist_blob(
    root: &Path,
    kind_dir: &str,
    ext: &str,
    blob_id: oqqwall_rust_core::BlobId,
    bytes: &[u8],
) -> Result<(String, u64), String> {
    let dir = root.join(kind_dir);
    fs::create_dir_all(&dir)
        .map_err(|err| format!("create blob dir failed: {}", err))?;
    let filename = format!("{}.{}", id128_hex(blob_id.0), ext);
    let path = dir.join(filename);
    let tmp_path = dir.join(format!("{}.{}.tmp", id128_hex(blob_id.0), ext));
    fs::write(&tmp_path, bytes).map_err(|err| format!("write blob failed: {}", err))?;
    if let Err(err) = fs::rename(&tmp_path, &path) {
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            fs::remove_file(&path)
                .map_err(|err| format!("cleanup blob failed: {}", err))?;
            fs::rename(&tmp_path, &path)
                .map_err(|err| format!("rename blob failed: {}", err))?;
        } else {
            return Err(format!("rename blob failed: {}", err));
        }
    }
    let size_bytes = bytes.len() as u64;
    Ok((path.to_string_lossy().to_string(), size_bytes))
}

fn render_blob_id(post_id: PostId) -> oqqwall_rust_core::BlobId {
    derive_blob_id(&[&post_id.to_be_bytes(), b"png"])
}

fn id128_hex(value: u128) -> String {
    format!("{:032x}", value)
}

async fn send_event(cmd_tx: &mpsc::Sender<Command>, event: Event) -> Result<(), String> {
    cmd_tx
        .send(Command::DriverEvent(event))
        .await
        .map_err(|_| "driver event send failed".to_string())
}

async fn send_render_failed(
    cmd_tx: &mpsc::Sender<Command>,
    post_id: PostId,
    attempt: u32,
    error: String,
) -> Result<(), String> {
    let retry_at_ms = now_ms().saturating_add(10_000);
    let event = RenderEvent::RenderFailed {
        post_id,
        attempt,
        retry_at_ms,
        error,
    };
    send_event(cmd_tx, Event::Render(event)).await
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Debug, Clone)]
struct FontBytes {
    path: PathBuf,
    bytes: Vec<u8>,
}

fn init_font_bytes_cache(font_dir: &Path) {
    if FONT_BYTES_CACHE.get().is_some() {
        return;
    }
    let mut fonts = Vec::new();
    if font_dir.exists() {
        if let Ok(entries) = fs::read_dir(font_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
                if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
                    continue;
                }
                match fs::read(&path) {
                    Ok(bytes) => fonts.push(FontBytes { path, bytes }),
                    Err(err) => {
                        debug_log!("font cache read failed: {} err={}", path.display(), err);
                    }
                }
            }
        }
    }
    debug_log!("font cache: disk_fonts={}", fonts.len());
    let _ = FONT_BYTES_CACHE.set(fonts);
}

fn font_bytes_cache() -> Option<&'static [FontBytes]> {
    FONT_BYTES_CACHE.get().map(|fonts| fonts.as_slice())
}

fn build_font_collection(font_dir: &Path) -> FontCollection {
    let mut asset_mgr = TypefaceFontProvider::new();
    let sys_mgr = skia_safe::FontMgr::new();
    debug_log!("font init: font_dir={}", font_dir.display());
    let embedded_count = register_embedded_fonts(&mut asset_mgr, &sys_mgr);
    debug_log!("font init: embedded_fonts={}", embedded_count);
    if let Some(fonts) = font_bytes_cache() {
        let mut disk_count = 0usize;
        for font in fonts {
            let ext = font
                .path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
                debug_log!("font skip non-ttf/otf: {}", font.path.display());
                continue;
            }
            if let Some(tf) = sys_mgr.new_from_data(&font.bytes, 0) {
                let alias = font_alias_for_path(&font.path);
                let family = tf.family_name();
                debug_log!(
                    "font disk load: path={} family={} alias={:?}",
                    font.path.display(),
                    family,
                    alias
                );
                register_typeface_with_alias(&mut asset_mgr, tf, alias);
                disk_count += 1;
            } else {
                debug_log!("font disk load failed: {}", font.path.display());
            }
        }
        debug_log!("font init: disk_fonts={}", disk_count);
    } else if font_dir.exists() {
        if let Ok(entries) = fs::read_dir(font_dir) {
            let mut disk_count = 0usize;
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
                if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
                    debug_log!("font skip non-ttf/otf: {}", path.display());
                    continue;
                }
                if let Ok(bytes) = fs::read(&path) {
                    if let Some(tf) = sys_mgr.new_from_data(&bytes, 0) {
                        let alias = font_alias_for_path(&path);
                        let family = tf.family_name();
                        debug_log!(
                            "font disk load: path={} family={} alias={:?}",
                            path.display(),
                            family,
                            alias
                        );
                        register_typeface_with_alias(&mut asset_mgr, tf, alias);
                        disk_count += 1;
                    } else {
                        debug_log!("font disk load failed: {}", path.display());
                    }
                } else {
                    debug_log!("font disk read failed: {}", path.display());
                }
            }
            debug_log!("font init: disk_fonts={}", disk_count);
        }
    } else {
        debug_log!("font dir not found: {}", font_dir.display());
    }
    let mut ordered_mgr = OrderedFontMgr::new();
    ordered_mgr.append(asset_mgr.clone());
    let mut fc = FontCollection::new();
    fc.set_asset_font_manager(Some(asset_mgr.into()));
    fc.set_default_font_manager_and_family_names(Some(ordered_mgr.into()), &FONT_FAMILIES);
    fc.disable_font_fallback();
    debug_log!(
        "font collection: managers={} fallback_enabled={}",
        fc.font_managers_count(),
        fc.font_fallback_enabled()
    );
    let debug_style = FontStyle::new(Weight::NORMAL, Width::NORMAL, Slant::Upright);
    let emoji_typefaces = fc.find_typefaces(&[EMOJI_FONT_ALIAS], debug_style);
    debug_log!(
        "font collection: emoji_alias_typefaces={}",
        emoji_typefaces.len()
    );
    for tf in emoji_typefaces {
        debug_log!(
            "font collection: emoji_alias family={} postscript={:?}",
            tf.family_name(),
            tf.post_script_name()
        );
    }
    let emoji_char: skia_safe::Unichar = 0x1F602;
    if let Some(tf) = fc.default_emoji_fallback(emoji_char, debug_style, "") {
        debug_log!(
            "font collection: emoji_fallback family={} postscript={:?}",
            tf.family_name(),
            tf.post_script_name()
        );
    } else {
        debug_log!("font collection: emoji_fallback none");
    }
    fc
}

fn register_embedded_fonts(
    asset_mgr: &mut TypefaceFontProvider,
    sys_mgr: &skia_safe::FontMgr,
) -> usize {
    let mut count = 0usize;
    for entry in embedded_resources::RESOURCES {
        if !entry.path.starts_with("fonts/") {
            continue;
        }
        let path = Path::new(entry.path);
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf") {
            debug_log!("embedded font skip non-ttf/otf: {}", entry.path);
            continue;
        }
        if let Some(tf) = sys_mgr.new_from_data(entry.bytes, 0usize) {
            let alias = font_alias_for_path(path);
            let family = tf.family_name();
            debug_log!(
                "embedded font load: path={} family={} alias={:?} bytes={}",
                entry.path,
                family,
                alias,
                entry.bytes.len()
            );
            register_typeface_with_alias(asset_mgr, tf, alias);
            count += 1;
        } else {
            debug_log!("embedded font load failed: {}", entry.path);
        }
    }
    count
}

fn register_typeface_with_alias(
    asset_mgr: &mut TypefaceFontProvider,
    typeface: Typeface,
    alias: Option<&'static str>,
) {
    if let Some(alias) = alias {
        asset_mgr.register_typeface(typeface.clone(), Some(alias));
    }
    asset_mgr.register_typeface(typeface, None);
}

fn font_alias_for_path(path: &Path) -> Option<&'static str> {
    let stem = path.file_stem()?.to_str()?;
    if stem.eq_ignore_ascii_case("AppleColorEmoji") {
        Some(EMOJI_FONT_ALIAS)
    } else {
        None
    }
}

fn embedded_bytes_for_path(path: &Path) -> Option<&'static [u8]> {
    let rel = match path_to_res_relative(path) {
        Some(rel) => rel,
        None => {
            debug_log!("embedded lookup: no res-relative path: {}", path.display());
            return None;
        }
    };
    let found = embedded_resource_bytes(&rel);
    if let Some(bytes) = found {
        debug_log!("embedded lookup hit: {} bytes={}", rel, bytes.len());
    } else {
        debug_log!("embedded lookup miss: {}", rel);
    }
    found
}

fn embedded_resource_bytes(path: &str) -> Option<&'static [u8]> {
    static RES_MAP: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    let map = RES_MAP.get_or_init(|| {
        let mut map = HashMap::new();
        debug_log!(
            "embedded resources init: count={}",
            embedded_resources::RESOURCES.len()
        );
        for entry in embedded_resources::RESOURCES {
            debug_log!(
                "embedded resource: {} bytes={}",
                entry.path,
                entry.bytes.len()
            );
            map.insert(entry.path, entry.bytes);
        }
        map
    });
    map.get(path).copied()
}

fn path_to_res_relative(path: &Path) -> Option<String> {
    if path.is_relative() {
        let rel = path.strip_prefix("res").ok()?;
        return path_to_slash(rel);
    }
    let res_dir = resolve_res_dir();
    let rel = path.strip_prefix(&res_dir).ok()?;
    path_to_slash(rel)
}

fn path_to_slash(path: &Path) -> Option<String> {
    let mut out = String::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_str()?;
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(part);
            }
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn resolve_res_dir() -> PathBuf {
    if let Ok(res_dir) = std::env::var("OQQWALL_RES_DIR") {
        let resolved = PathBuf::from(res_dir);
        debug_log!("res dir from env: {}", resolved.display());
        return resolved;
    }
    let cwd_candidate = PathBuf::from("res");
    if cwd_candidate.exists() {
        debug_log!("res dir from cwd: {}", cwd_candidate.display());
        return cwd_candidate;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidates = [
                exe_dir.join("res"),
                exe_dir.join("..").join("res"),
                exe_dir.join("..").join("..").join("res"),
            ];
            for candidate in candidates {
                if candidate.exists() {
                    debug_log!("res dir from exe: {}", candidate.display());
                    return candidate;
                }
            }
        }
    }
    debug_log!("res dir fallback: {}", cwd_candidate.display());
    cwd_candidate
}

fn resolve_font_dir() -> PathBuf {
    resolve_res_dir().join("fonts")
}

async fn render_png_async(
    draft: &Draft,
    header: &HeaderInfo,
    image_sources: &RenderImageSources,
    config: &RendererRuntimeConfig,
) -> Result<Vec<u8>, String> {
    let draft = draft.clone();
    let header = header.clone();
    let image_sources = image_sources.clone();
    let config = config.clone();
    tokio::task::spawn_blocking(move || render_png(&draft, &header, &image_sources, &config))
        .await
        .map_err(|err| format!("png task failed: {}", err))?
}
