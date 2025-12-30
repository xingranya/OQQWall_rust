use oqqwall_rust_core::event::{Event, RenderEvent, RenderFormat};
use oqqwall_rust_core::{derive_blob_id, Command};
use oqqwall_rust_drivers::napcat::{spawn_napcat_ws, NapCatRuntimeConfig};
use oqqwall_rust_drivers::qzone::{spawn_qzone_sender, QzoneRuntimeConfig};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::config::AppConfig;
use crate::engine::EngineHandle;

pub fn spawn_napcat_drivers(handle: &EngineHandle, config: &AppConfig) {
    let runtime = NapCatRuntimeConfig {
        napcat: config.napcat.clone(),
        audit_group_id: config.audit_group_id.clone(),
        default_group_id: config.default_group_id.clone(),
        tz_offset_minutes: config.tz_offset_minutes,
    };
    let bus_rx = handle.subscribe();
    spawn_napcat_ws(handle.cmd_tx.clone(), bus_rx, runtime);
    spawn_render_stub(handle);
    let runtime = QzoneRuntimeConfig {
        napcat: config.napcat.clone(),
    };
    spawn_qzone_sender(handle.cmd_tx.clone(), handle.subscribe(), runtime);
}

fn spawn_render_stub(handle: &EngineHandle) -> JoinHandle<()> {
    let mut rx = handle.subscribe();
    let cmd_tx = handle.cmd_tx.clone();

    tokio::spawn(async move {
        loop {
            let env = match rx.recv().await {
                Ok(env) => env,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };

            if let Event::Render(RenderEvent::RenderRequested { post_id, format, .. }) = env.event {
                let blob_id = render_blob_id(post_id, format);
                let event = match format {
                    RenderFormat::Svg => Event::Render(RenderEvent::SvgReady { post_id, blob_id }),
                    RenderFormat::Png => Event::Render(RenderEvent::PngReady { post_id, blob_id }),
                };
                if cmd_tx.send(Command::DriverEvent(event)).await.is_err() {
                    break;
                }
            }
        }
    })
}

fn render_blob_id(post_id: oqqwall_rust_core::PostId, format: RenderFormat) -> oqqwall_rust_core::BlobId {
    let tag = match format {
        RenderFormat::Svg => b"svg",
        RenderFormat::Png => b"png",
    };
    derive_blob_id(&[&post_id.to_be_bytes(), tag])
}
