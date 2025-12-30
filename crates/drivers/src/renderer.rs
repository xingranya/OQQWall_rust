use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use oqqwall_rust_core::event::{BlobEvent, Event, RenderEvent, RenderFormat};
use oqqwall_rust_core::{derive_blob_id, Command, Draft, DraftBlock, PostId, StateView};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::sync::broadcast::error::RecvError;

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
pub struct RendererRuntimeConfig {
    pub blob_root: PathBuf,
    pub canvas_width_px: u32,
    pub max_height_px: u32,
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
        }
    }
}

#[derive(Debug, Clone)]
struct HeaderInfo {
    group_id: String,
    user_id: String,
    post_id_hex: String,
}

#[derive(Debug, Clone)]
enum BlockKind {
    Text { lines: Vec<String> },
    Attachment { label: String },
}

#[derive(Debug, Clone)]
struct BlockLayout {
    y: u32,
    height: u32,
    kind: BlockKind,
}

pub fn spawn_renderer(
    cmd_tx: mpsc::Sender<Command>,
    bus_rx: broadcast::Receiver<oqqwall_rust_core::EventEnvelope>,
    config: RendererRuntimeConfig,
) -> JoinHandle<()> {
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
                format,
                attempt,
                ..
            }) = env.event
            {
                if let Err(err) =
                    handle_render_request(&cmd_tx, &state, post_id, format, attempt, &config)
                        .await
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
    format: RenderFormat,
    attempt: u32,
    config: &RendererRuntimeConfig,
) -> Result<(), String> {
    let draft = match state.drafts.get(&post_id) {
        Some(draft) => draft.clone(),
        None => {
            return send_render_failed(
                cmd_tx,
                post_id,
                format,
                attempt,
                "missing draft for render".to_string(),
            )
            .await;
        }
    };

    let header = extract_header(state, post_id);
    let svg = render_svg(&draft, &header, config);

    let bytes = match format {
        RenderFormat::Svg => svg.into_bytes(),
        RenderFormat::Png => match render_png_async(svg).await {
            Ok(bytes) => bytes,
            Err(err) => {
                return send_render_failed(cmd_tx, post_id, format, attempt, err).await;
            }
        },
    };

    let blob_id = render_blob_id(post_id, format);
    let (kind_dir, ext) = match format {
        RenderFormat::Svg => ("svg", "svg"),
        RenderFormat::Png => ("png", "png"),
    };
    let (path, size_bytes) = persist_blob(&config.blob_root, kind_dir, ext, blob_id, &bytes)?;

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

    let render_event = match format {
        RenderFormat::Svg => RenderEvent::SvgReady { post_id, blob_id },
        RenderFormat::Png => RenderEvent::PngReady { post_id, blob_id },
    };
    send_event(cmd_tx, Event::Render(render_event)).await?;

    Ok(())
}

fn extract_header(state: &StateView, post_id: PostId) -> HeaderInfo {
    let mut group_id = "unknown".to_string();
    let mut user_id = "unknown".to_string();
    if let Some(ingress_ids) = state.post_ingress.get(&post_id) {
        for ingress_id in ingress_ids {
            if let Some(meta) = state.ingress_meta.get(ingress_id) {
                group_id = meta.group_id.clone();
                user_id = meta.user_id.clone();
                break;
            }
        }
    }
    HeaderInfo {
        group_id,
        user_id,
        post_id_hex: id128_hex(post_id.0),
    }
}

fn render_svg(draft: &Draft, header: &HeaderInfo, config: &RendererRuntimeConfig) -> String {
    let padding = 20u32;
    let block_gap = 12u32;
    let bubble_padding = 10u32;
    let font_size = 14u32;
    let line_height = 20u32;
    let title_size = 20u32;
    let meta_size = 12u32;
    let header_gap = 12u32;

    let content_width = config.canvas_width_px.saturating_sub(padding.saturating_mul(2));
    let header_title_y = padding + title_size;
    let header_meta_y = header_title_y + meta_size + 6;
    let header_meta2_y = header_meta_y + meta_size + 4;
    let header_height = header_meta2_y.saturating_add(meta_size).saturating_sub(padding);
    let mut cursor_y = padding + header_height + header_gap;

    let max_chars = max_chars_per_line(content_width, bubble_padding, font_size);
    let mut blocks = Vec::new();

    for block in &draft.blocks {
        let (height, kind) = match block {
            DraftBlock::Paragraph { text } => {
                let lines = wrap_text(text, max_chars);
                let height = bubble_padding
                    .saturating_mul(2)
                    .saturating_add(line_height.saturating_mul(lines.len() as u32));
                (height, BlockKind::Text { lines })
            }
            DraftBlock::Attachment { kind, .. } => {
                let label = attachment_label(*kind);
                let height = match kind {
                    oqqwall_rust_core::MediaKind::Image => 180,
                    _ => 90,
                };
                (height, BlockKind::Attachment { label })
            }
        };

        let mut next_bottom = cursor_y.saturating_add(height).saturating_add(padding);
        if !blocks.is_empty() {
            next_bottom = next_bottom.saturating_add(block_gap);
        }
        if next_bottom > config.max_height_px {
            let trunc_lines = vec!["... truncated ...".to_string()];
            let trunc_height = bubble_padding
                .saturating_mul(2)
                .saturating_add(line_height);
            let trunc_bottom = cursor_y.saturating_add(trunc_height).saturating_add(padding);
            if trunc_bottom <= config.max_height_px {
                blocks.push(BlockLayout {
                    y: cursor_y,
                    height: trunc_height,
                    kind: BlockKind::Text {
                        lines: trunc_lines,
                    },
                });
                cursor_y = cursor_y.saturating_add(trunc_height).saturating_add(block_gap);
            }
            break;
        }

        blocks.push(BlockLayout { y: cursor_y, height, kind });
        cursor_y = cursor_y.saturating_add(height).saturating_add(block_gap);
    }

    if !blocks.is_empty() {
        cursor_y = cursor_y.saturating_sub(block_gap);
    }

    let canvas_height = cursor_y.saturating_add(padding).max(padding + header_height + header_gap);
    let background_height = canvas_height.min(config.max_height_px).max(1);

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        config.canvas_width_px,
        background_height,
        config.canvas_width_px,
        background_height
    ));
    out.push_str("<style>");
    out.push_str(".title{font:");
    out.push_str(&format!("{}px \"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;fill:#000;", title_size));
    out.push_str(".meta{font:");
    out.push_str(&format!("{}px \"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;fill:#666;", meta_size));
    out.push_str(".body{font:");
    out.push_str(&format!("{}px \"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;fill:#000;", font_size));
    out.push_str(".muted{font:");
    out.push_str(&format!("{}px \"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;fill:#666;", font_size));
    out.push_str("</style>");
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#f2f2f2\" rx=\"12\" />",
        config.canvas_width_px, background_height
    ));

    out.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"title\">OQQWall</text>",
        padding, header_title_y
    ));
    out.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"meta\">Group: {}  User: {}</text>",
        padding,
        header_meta_y,
        escape_xml(&header.group_id),
        escape_xml(&header.user_id)
    ));
    out.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"meta\">Post: {}</text>",
        padding,
        header_meta2_y,
        escape_xml(&header.post_id_hex)
    ));
    out.push_str(&format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e0e0e0\" stroke-width=\"1\" />",
        padding,
        padding + header_height + header_gap / 2,
        padding + content_width,
        padding + header_height + header_gap / 2
    ));

    for block in blocks {
        match block.kind {
            BlockKind::Text { lines } => {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" rx=\"8\" />",
                    padding, block.y, content_width, block.height
                ));
                let text_x = padding + bubble_padding;
                let text_y = block.y + bubble_padding + font_size;
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" class=\"body\">",
                    text_x, text_y
                ));
                for (idx, line) in lines.iter().enumerate() {
                    let escaped = escape_xml(line);
                    if idx == 0 {
                        out.push_str(&format!("<tspan x=\"{}\" dy=\"0\">{}</tspan>", text_x, escaped));
                    } else {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                            text_x, line_height, escaped
                        ));
                    }
                }
                out.push_str("</text>");
            }
            BlockKind::Attachment { label } => {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" rx=\"8\" />",
                    padding, block.y, content_width, block.height
                ));
                let text_x = padding + bubble_padding;
                let text_y = block.y + bubble_padding + font_size;
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" class=\"muted\">{}</text>",
                    text_x,
                    text_y,
                    escape_xml(&label)
                ));
            }
        }
    }

    out.push_str("</svg>");
    out
}

fn max_chars_per_line(content_width: u32, bubble_padding: u32, font_size: u32) -> usize {
    let text_width = content_width.saturating_sub(bubble_padding.saturating_mul(2));
    let approx_char_width = (font_size.saturating_mul(2)).saturating_div(3).max(6);
    let max_chars = text_width.saturating_div(approx_char_width).max(1);
    max_chars as usize
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.chars().count() <= max_chars {
            lines.push(raw_line.to_string());
            continue;
        }
        let mut current = String::new();
        let mut count = 0usize;
        for ch in raw_line.chars() {
            current.push(ch);
            count += 1;
            if count >= max_chars {
                lines.push(current.clone());
                current.clear();
                count = 0;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn attachment_label(kind: oqqwall_rust_core::MediaKind) -> String {
    match kind {
        oqqwall_rust_core::MediaKind::Image => "Attachment: Image".to_string(),
        oqqwall_rust_core::MediaKind::Video => "Attachment: Video".to_string(),
        oqqwall_rust_core::MediaKind::File => "Attachment: File".to_string(),
        oqqwall_rust_core::MediaKind::Audio => "Attachment: Audio".to_string(),
        oqqwall_rust_core::MediaKind::Other => "Attachment: Other".to_string(),
    }
}

fn escape_xml(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
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
    fs::write(&path, bytes).map_err(|err| format!("write blob failed: {}", err))?;
    let size_bytes = bytes.len() as u64;
    Ok((path.to_string_lossy().to_string(), size_bytes))
}

fn render_blob_id(post_id: PostId, format: RenderFormat) -> oqqwall_rust_core::BlobId {
    let tag = match format {
        RenderFormat::Svg => b"svg",
        RenderFormat::Png => b"png",
    };
    derive_blob_id(&[&post_id.to_be_bytes(), tag])
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
    format: RenderFormat,
    attempt: u32,
    error: String,
) -> Result<(), String> {
    let retry_at_ms = now_ms().saturating_add(10_000);
    let event = RenderEvent::RenderFailed {
        post_id,
        format,
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

async fn render_png_async(svg: String) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || render_png(&svg))
        .await
        .map_err(|err| format!("png task failed: {}", err))?
}

fn render_png(svg: &str) -> Result<Vec<u8>, String> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|err| format!("parse svg failed: {}", err))?;
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| "pixmap alloc failed".to_string())?;
    let mut pixmap_mut = pixmap.as_mut();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap_mut,
    );
    pixmap
        .encode_png()
        .map_err(|err| format!("encode png failed: {}", err))
}
