mod engine;
mod connect;
mod config;
mod oobe;
mod status;

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

use engine::Engine;
use connect::spawn_napcat_drivers;
use config::AppConfig;
use oqqwall_rust_core::Command;
use tokio::time::{sleep, Duration};
use std::env;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    debug_log!("debug build: args={:?}", args);
    if args.len() > 1 && (args[1] == "oobe" || args[1] == "--oobe") {
        if let Err(err) = oobe::run(&args[1..]) {
            eprintln!("{}", err);
            std::process::exit(1);
        }
        return;
    }

    println!("系统已启动");
    let app_config = AppConfig::load().expect("failed to load config.json");
    debug_log!(
        "config loaded: groups={} tz_offset_minutes={} fallback_napcat={}",
        app_config.groups.len(),
        app_config.tz_offset_minutes,
        app_config.fallback_napcat.is_some()
    );
    let core_config = app_config.build_core_config();
    debug_log!(
        "core config: default_wait_ms={} default_min_interval_ms={} default_max_queue={} groups={}",
        core_config.default_process_waittime_ms,
        core_config.default_min_interval_ms,
        core_config.default_max_queue,
        core_config.groups.len()
    );
    let data_dir = env::var("OQQWALL_DATA_DIR").unwrap_or_else(|_| "data".to_string());
    let (engine, handle) =
        Engine::new(core_config, &data_dir).expect("failed to init engine");
    debug_log!("engine created: data_dir={}", data_dir);
    let _status = status::spawn_status_logger(&handle);
    debug_log!("status logger spawned");
    spawn_napcat_drivers(&handle, &app_config);
    debug_log!("drivers spawned");

    let cmd_tx = handle.cmd_tx.clone();
    let tz_offset_minutes = app_config.tz_offset_minutes;
    tokio::spawn(async move {
        debug_log!("tick loop started");
        loop {
            let now_ms = now_ms();
            let tick = oqqwall_rust_core::TickCommand {
                now_ms,
                tz_offset_minutes,
            };
            if cmd_tx.send(Command::Tick(tick)).await.is_err() {
                debug_log!("tick loop stopped: cmd channel closed");
                break;
            }
            sleep(Duration::from_secs(1)).await;
        }
    });

    debug_log!("engine run loop starting");
    engine.run().await;
    debug_log!("engine run loop exited");
}

fn now_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}
