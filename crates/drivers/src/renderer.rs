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
    Image { href: Option<String>, clip_id: String },
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
struct BlockLayout {
    x: u32,
    y: u32,
    width: u32,
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
    let spacing_lg = 10u32;
    let spacing_xxl = 20u32;
    let bubble_pad_x = 8u32;
    let bubble_pad_y = 4u32;
    let font_size = 14u32;
    let line_height = 21u32;
    let title_size = 24u32;
    let meta_size = 12u32;
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

    let content_width = config.canvas_width_px.saturating_sub(padding.saturating_mul(2));
    let header_x = padding;
    let header_y = padding;
    let header_text_x = header_x + avatar_size + header_gap;
    let header_text_width = content_width.saturating_sub(avatar_size + header_gap);

    let title_text = if header.user_id == "unknown" {
        "OQQWall".to_string()
    } else {
        format!("User {}", header.user_id)
    };
    let title_text = truncate_text(&title_text, header_text_width, title_size);
    let meta_text = truncate_text(
        &format!("Group {}  Post {}", header.group_id, header.post_id_hex),
        header_text_width,
        meta_size,
    );
    let title_y = header_y + title_size;
    let meta_y = title_y + meta_size + 4;
    let header_height = avatar_size.max(meta_y + meta_size - header_y);

    let mut cursor_y = header_y + header_height + spacing_xxl;
    let mut blocks = Vec::new();
    let mut clip_defs = Vec::new();
    let mut image_index = 0usize;

    for block in &draft.blocks {
        let layout = match block {
            DraftBlock::Paragraph { text } => {
                let max_text_w = content_width.saturating_sub(bubble_pad_x * 2).max(1);
                let lines = wrap_text(text, max_text_w, font_size);
                let mut max_line_w = 0u32;
                for line in &lines {
                    max_line_w = max_line_w.max(estimate_text_width(line, font_size));
                }
                let bubble_w = (max_line_w + bubble_pad_x * 2).min(content_width).max(1);
                let height = bubble_pad_y * 2 + line_height * lines.len() as u32;
                BlockLayout {
                    x: padding,
                    y: cursor_y,
                    width: bubble_w,
                    height,
                    kind: BlockKind::Text { lines },
                }
            }
            DraftBlock::Attachment { kind, reference } => {
                let href = reference_url(reference);
                match *kind {
                    oqqwall_rust_core::MediaKind::Image => {
                        let width = (content_width / 2).max(1);
                        let height = (width.saturating_mul(3).saturating_div(4))
                            .min(300)
                            .max(1);
                        let clip_id = format!("img-clip-{}", image_index);
                        image_index += 1;
                        clip_defs.push(format!(
                            "<clipPath id=\"{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" /></clipPath>",
                            clip_id, padding, cursor_y, width, height, radius_lg
                        ));
                        BlockLayout {
                            x: padding,
                            y: cursor_y,
                            width,
                            height,
                            kind: BlockKind::Image { href, clip_id },
                        }
                    }
                    oqqwall_rust_core::MediaKind::File => {
                        let width = content_width.min(320).max(1);
                        let text_max_w = width
                            .saturating_sub(file_padding * 2 + file_icon_size + file_icon_gap)
                            .max(1);
                        let filename = href
                            .as_deref()
                            .and_then(extract_filename)
                            .unwrap_or_else(|| "Unknown file".to_string());
                        let mut name_lines = wrap_text(&filename, text_max_w, font_size);
                        name_lines = limit_lines(name_lines, 2, text_max_w, font_size);
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
                        if let Some(detail) = href.as_deref().and_then(extract_filename) {
                            let detail_line = truncate_text(&detail, text_max_w, font_size);
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

        let layout_height = layout.height;
        let mut next_bottom = layout
            .y
            .saturating_add(layout_height)
            .saturating_add(padding);
        if !blocks.is_empty() {
            next_bottom = next_bottom.saturating_add(spacing_lg);
        }
        if next_bottom > config.max_height_px {
            let trunc_lines = vec!["... truncated ...".to_string()];
            let mut max_line_w = 0u32;
            for line in &trunc_lines {
                max_line_w = max_line_w.max(estimate_text_width(line, font_size));
            }
            let bubble_w = (max_line_w + bubble_pad_x * 2).min(content_width).max(1);
            let trunc_height = bubble_pad_y * 2 + line_height;
            let trunc_bottom = cursor_y.saturating_add(trunc_height).saturating_add(padding);
            if trunc_bottom <= config.max_height_px {
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
    let background_height = canvas_height.min(config.max_height_px).max(1);

    let avatar_url = if header.user_id == "unknown" {
        None
    } else {
        Some(format!(
            "https://qlogo2.store.qq.com/qzone/{0}/{0}/640",
            header.user_id
        ))
    };
    let avatar_cx = header_x + avatar_size / 2;
    let avatar_cy = header_y + avatar_size / 2;

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        config.canvas_width_px,
        background_height,
        config.canvas_width_px,
        background_height
    ));
    out.push_str("<defs>");
    out.push_str("<filter id=\"shadow-sm\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\">");
    out.push_str("<feDropShadow dx=\"0\" dy=\"0\" stdDeviation=\"2\" flood-color=\"#000\" flood-opacity=\"0.10\" />");
    out.push_str("</filter>");
    out.push_str("<filter id=\"shadow-md\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\">");
    out.push_str("<feDropShadow dx=\"0\" dy=\"0\" stdDeviation=\"3\" flood-color=\"#000\" flood-opacity=\"0.20\" />");
    out.push_str("</filter>");
    out.push_str("<filter id=\"shadow-lg\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\">");
    out.push_str("<feDropShadow dx=\"0\" dy=\"0\" stdDeviation=\"4\" flood-color=\"#000\" flood-opacity=\"0.30\" />");
    out.push_str("</filter>");
    out.push_str(&format!(
        "<clipPath id=\"avatar-clip\"><circle cx=\"{}\" cy=\"{}\" r=\"{}\" /></clipPath>",
        avatar_cx,
        avatar_cy,
        avatar_size / 2
    ));
    for def in clip_defs {
        out.push_str(&def);
    }
    out.push_str("</defs>");
    out.push_str("<style>");
    out.push_str(".title{font-family:\"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;font-size:");
    out.push_str(&format!("{}px;font-weight:600;fill:#000;", title_size));
    out.push_str(".meta{font-family:\"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;font-size:");
    out.push_str(&format!("{}px;fill:#666;", meta_size));
    out.push_str(".body{font-family:\"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;font-size:");
    out.push_str(&format!("{}px;fill:#000;", font_size));
    out.push_str(".muted{font-family:\"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;font-size:");
    out.push_str(&format!("{}px;fill:#888;", meta_size));
    out.push_str(".file-meta{font-family:\"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;font-size:11px;fill:#888;}");
    out.push_str(".icon-text{font-family:\"PingFang SC\", \"Microsoft YaHei\", Arial, sans-serif;font-size:11px;fill:#666;}");
    out.push_str("</style>");
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"#f2f2f2\" rx=\"12\" />",
        config.canvas_width_px, background_height
    ));

    out.push_str(&format!(
        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" filter=\"url(#shadow-lg)\" />",
        avatar_cx,
        avatar_cy,
        avatar_size / 2
    ));
    if let Some(url) = avatar_url {
        out.push_str(&format!(
            "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\" clip-path=\"url(#avatar-clip)\" />",
            header_x,
            header_y,
            avatar_size,
            avatar_size,
            escape_xml(&url)
        ));
    }
    out.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"title\">{}</text>",
        header_text_x,
        title_y,
        escape_xml(&title_text)
    ));
    out.push_str(&format!(
        "<text x=\"{}\" y=\"{}\" class=\"meta\">{}</text>",
        header_text_x,
        meta_y,
        escape_xml(&meta_text)
    ));
    out.push_str(&format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"#e0e0e0\" stroke-width=\"1\" />",
        padding,
        header_y + header_height + spacing_xxl / 2,
        padding + content_width,
        header_y + header_height + spacing_xxl / 2
    ));

    for block in blocks {
        match block.kind {
            BlockKind::Text { lines } => {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" rx=\"{}\" filter=\"url(#shadow-sm)\" />",
                    block.x, block.y, block.width, block.height, radius_lg
                ));
                let text_x = block.x + bubble_pad_x;
                let text_y = block.y + bubble_pad_y + font_size;
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" class=\"body\">",
                    text_x, text_y
                ));
                for (idx, line) in lines.iter().enumerate() {
                    let escaped = escape_xml(line);
                    if idx == 0 {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"0\">{}</tspan>",
                            text_x, escaped
                        ));
                    } else {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                            text_x, line_height, escaped
                        ));
                    }
                }
                out.push_str("</text>");
            }
            BlockKind::Image { href, clip_id } => {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" rx=\"{}\" filter=\"url(#shadow-md)\" />",
                    block.x, block.y, block.width, block.height, radius_lg
                ));
                if let Some(url) = href {
                    out.push_str(&format!(
                        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\" preserveAspectRatio=\"xMidYMid slice\" clip-path=\"url(#{})\" />",
                        block.x,
                        block.y,
                        block.width,
                        block.height,
                        escape_xml(&url),
                        clip_id
                    ));
                } else {
                    let text_x = block.x + bubble_pad_x;
                    let text_y = block.y + bubble_pad_y + font_size;
                    out.push_str(&format!(
                        "<text x=\"{}\" y=\"{}\" class=\"muted\">Image</text>",
                        text_x, text_y
                    ));
                }
            }
            BlockKind::MediaCard {
                lines,
                icon_text,
                media_kind,
            } => {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" rx=\"{}\" filter=\"url(#shadow-sm)\" />",
                    block.x, block.y, block.width, block.height, radius_lg
                ));
                let icon_x = block.x + card_padding;
                let icon_y = block.y + (block.height - card_icon_size) / 2;
                if matches!(media_kind, oqqwall_rust_core::MediaKind::Video) {
                    let cx = icon_x + card_icon_size / 2;
                    let cy = icon_y + card_icon_size / 2;
                    out.push_str(&format!(
                        "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#f2f2f2\" stroke=\"#e0e0e0\" />",
                        cx,
                        cy,
                        card_icon_size / 2
                    ));
                    let tri_x = icon_x + card_icon_size / 3;
                    let tri_y = icon_y + card_icon_size / 4;
                    let tri_w = card_icon_size / 2;
                    let tri_h = card_icon_size / 2;
                    out.push_str(&format!(
                        "<path d=\"M {} {} L {} {} L {} {} Z\" fill=\"#007aff\" />",
                        tri_x,
                        tri_y,
                        tri_x,
                        tri_y + tri_h,
                        tri_x + tri_w,
                        tri_y + tri_h / 2
                    ));
                } else {
                    out.push_str(&format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" fill=\"#f2f2f2\" stroke=\"#e0e0e0\" />",
                        icon_x, icon_y, card_icon_size, card_icon_size
                    ));
                    out.push_str(&format!(
                        "<text x=\"{}\" y=\"{}\" class=\"icon-text\" text-anchor=\"middle\" dominant-baseline=\"middle\">{}</text>",
                        icon_x + card_icon_size / 2,
                        icon_y + card_icon_size / 2,
                        escape_xml(&icon_text)
                    ));
                }
                let text_x = block.x + card_padding + card_icon_size + card_icon_gap;
                let text_y = block.y + card_padding + font_size;
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" class=\"body\">",
                    text_x, text_y
                ));
                for (idx, line) in lines.iter().enumerate() {
                    let escaped = escape_xml(line);
                    if idx == 0 {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"0\">{}</tspan>",
                            text_x, escaped
                        ));
                    } else {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                            text_x, card_line_height, escaped
                        ));
                    }
                }
                out.push_str("</text>");
            }
            BlockKind::FileCard {
                name_lines,
                meta_line,
                icon_text,
            } => {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffffff\" stroke=\"#e0e0e0\" rx=\"{}\" filter=\"url(#shadow-sm)\" />",
                    block.x, block.y, block.width, block.height, radius_lg
                ));
                let icon_x = block.x + block.width.saturating_sub(file_padding + file_icon_size);
                let icon_y = block.y + (block.height - file_icon_size) / 2;
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"4\" fill=\"#f2f2f2\" stroke=\"#e0e0e0\" />",
                    icon_x, icon_y, file_icon_size, file_icon_size
                ));
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" class=\"icon-text\" text-anchor=\"middle\" dominant-baseline=\"middle\">{}</text>",
                    icon_x + file_icon_size / 2,
                    icon_y + file_icon_size / 2,
                    escape_xml(&icon_text)
                ));
                let text_x = block.x + file_padding;
                let name_height = name_lines.len() as u32 * file_line_height;
                let meta_height = if meta_line.is_some() {
                    file_meta_height + file_meta_gap
                } else {
                    0
                };
                let text_height = name_height + meta_height;
                let content_height = file_icon_size.max(text_height);
                let text_y = block.y + file_padding + (content_height - text_height) / 2 + font_size;
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" class=\"body\">",
                    text_x, text_y
                ));
                for (idx, line) in name_lines.iter().enumerate() {
                    let escaped = escape_xml(line);
                    if idx == 0 {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"0\">{}</tspan>",
                            text_x, escaped
                        ));
                    } else {
                        out.push_str(&format!(
                            "<tspan x=\"{}\" dy=\"{}\">{}</tspan>",
                            text_x, file_line_height, escaped
                        ));
                    }
                }
                out.push_str("</text>");
                if let Some(meta) = meta_line {
                    let meta_y = text_y + name_height + file_meta_gap + file_meta_height;
                    out.push_str(&format!(
                        "<text x=\"{}\" y=\"{}\" class=\"file-meta\">{}</text>",
                        text_x,
                        meta_y,
                        escape_xml(&meta)
                    ));
                }
            }
        }
    }

    out.push_str("</svg>");
    out
}

fn reference_url(reference: &oqqwall_rust_core::MediaReference) -> Option<String> {
    match reference {
        oqqwall_rust_core::MediaReference::RemoteUrl { url } => Some(url.clone()),
        oqqwall_rust_core::MediaReference::Blob { .. } => None,
    }
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

fn truncate_text(text: &str, max_width: u32, font_size: u32) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0u32;
    for ch in text.chars() {
        let ch_width = estimate_char_width(ch, font_size);
        if width + ch_width > max_width {
            let ellipsis_width = estimate_text_width("...", font_size);
            if width + ellipsis_width <= max_width && !out.is_empty() {
                out.push_str("...");
            }
            return out;
        }
        out.push(ch);
        width = width.saturating_add(ch_width);
    }
    out
}

fn limit_lines(
    mut lines: Vec<String>,
    max_lines: usize,
    max_width: u32,
    font_size: u32,
) -> Vec<String> {
    if lines.len() <= max_lines {
        return lines;
    }
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        let padded = format!("{}...", last);
        *last = truncate_text(&padded, max_width, font_size);
    }
    lines
}

fn wrap_text(text: &str, max_width: u32, font_size: u32) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current: Vec<char> = Vec::new();
        let mut current_width = 0u32;
        let mut last_break: Option<usize> = None;
        for ch in raw_line.chars() {
            let ch_width = estimate_char_width(ch, font_size);
            if current_width + ch_width > max_width && !current.is_empty() {
                if let Some(break_idx) = last_break {
                    let line: String = current[..break_idx].iter().collect();
                    lines.push(line.trim_end().to_string());
                    let mut remainder: Vec<char> = current[break_idx..].iter().copied().collect();
                    while remainder.first().map(|c| c.is_whitespace()).unwrap_or(false) {
                        remainder.remove(0);
                    }
                    current = remainder;
                    current_width = estimate_text_width(&current.iter().collect::<String>(), font_size);
                    last_break = None;
                } else {
                    let line: String = current.iter().collect();
                    lines.push(line);
                    current.clear();
                    current_width = 0;
                    last_break = None;
                }
            }
            current.push(ch);
            current_width = current_width.saturating_add(ch_width);
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

fn is_break_char(ch: char) -> bool {
    if ch.is_whitespace() {
        return true;
    }
    matches!(
        ch,
        '-' | '/' | '_' | '.' | ',' | ';' | ':' | '?' | '!' | '，' | '。' | '；' | '、' | '：'
    )
}

fn estimate_char_width(ch: char, font_size: u32) -> u32 {
    if ch.is_ascii() {
        if ch.is_whitespace() {
            return font_size.saturating_mul(1).saturating_div(3).max(3);
        }
        if ch.is_ascii_punctuation() {
            return font_size.saturating_mul(1).saturating_div(2).max(4);
        }
        return font_size.saturating_mul(3).saturating_div(5).max(6);
    }
    font_size
}

fn estimate_text_width(text: &str, font_size: u32) -> u32 {
    text.chars()
        .map(|ch| estimate_char_width(ch, font_size))
        .sum()
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
