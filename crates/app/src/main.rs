mod engine;
mod connect;
mod config;

use engine::Engine;
use connect::spawn_napcat_drivers;
use config::AppConfig;
use oqqwall_rust_core::Command;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() {
    let app_config = AppConfig::load().expect("failed to load config.json");
    let core_config = app_config.build_core_config();
    let (engine, handle) = Engine::new(core_config);
    spawn_napcat_drivers(&handle, &app_config);

    let cmd_tx = handle.cmd_tx.clone();
    let tz_offset_minutes = app_config.tz_offset_minutes;
    tokio::spawn(async move {
        loop {
            let now_ms = now_ms();
            let tick = oqqwall_rust_core::TickCommand {
                now_ms,
                tz_offset_minutes,
            };
            if cmd_tx.send(Command::Tick(tick)).await.is_err() {
                break;
            }
            sleep(Duration::from_secs(1)).await;
        }
    });

    engine.run().await;
}

fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
