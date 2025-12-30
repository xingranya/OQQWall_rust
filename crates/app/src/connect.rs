use oqqwall_rust_drivers::media_fetcher::{spawn_media_fetcher, MediaFetcherRuntimeConfig};
use oqqwall_rust_drivers::napcat::{spawn_napcat_ws, NapCatRuntimeConfig};
use oqqwall_rust_drivers::renderer::{spawn_renderer, RendererRuntimeConfig};
use oqqwall_rust_drivers::qzone::{spawn_qzone_sender, QzoneRuntimeConfig};
use std::collections::HashMap;

use crate::config::AppConfig;
use crate::engine::EngineHandle;

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

pub fn spawn_napcat_drivers(handle: &EngineHandle, config: &AppConfig) {
    debug_log!(
        "spawning drivers: groups={} tz_offset_minutes={}",
        config.groups.len(),
        config.tz_offset_minutes
    );
    for group in &config.groups {
        let runtime = NapCatRuntimeConfig {
            napcat: group.napcat.clone(),
            audit_group_id: group.audit_group_id.clone(),
            group_id: group.group_id.clone(),
            tz_offset_minutes: config.tz_offset_minutes,
        };
        let ws_log = ws_url_for_log(&runtime.napcat.ws_url);
        debug_log!(
            "spawn napcat ws: group_id={} audit_group_id={:?} ws_url={} token_present={}",
            runtime.group_id,
            runtime.audit_group_id,
            ws_log,
            runtime.napcat.access_token.is_some()
        );
        let bus_rx = handle.subscribe();
        spawn_napcat_ws(handle.cmd_tx.clone(), bus_rx, runtime);
    }
    debug_log!("spawn media fetcher");
    spawn_media_fetcher(
        handle.cmd_tx.clone(),
        handle.subscribe(),
        MediaFetcherRuntimeConfig::default(),
    );
    debug_log!("spawn renderer");
    spawn_renderer(
        handle.cmd_tx.clone(),
        handle.subscribe(),
        RendererRuntimeConfig::default(),
    );
    let mut napcat_by_group = HashMap::new();
    for group in &config.groups {
        napcat_by_group.insert(group.group_id.clone(), group.napcat.clone());
    }
    let runtime = QzoneRuntimeConfig {
        napcat_by_group,
        default_napcat: config.fallback_napcat.clone(),
        #[cfg(debug_assertions)]
        use_virt_qzone: config.dev_config.use_virt_qzone,
    };
    debug_log!(
        "spawn qzone sender: default_napcat={}",
        runtime.default_napcat.is_some()
    );
    spawn_qzone_sender(handle.cmd_tx.clone(), handle.subscribe(), runtime);
}

#[cfg(debug_assertions)]
fn ws_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}
