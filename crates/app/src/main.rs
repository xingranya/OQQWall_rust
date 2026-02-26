mod config;
mod connect;
mod engine;
mod oobe;
mod status;
mod telemetry;
mod web_api;
mod webview;

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {
        oqqwall_rust_infra::debug_log::log(format_args!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

use config::AppConfig;
use connect::spawn_napcat_drivers;
use engine::Engine;
use oqqwall_rust::tui::oqqwall_tui;
use oqqwall_rust_core::Command;
use std::env;
use std::io::IsTerminal;
use tokio::time::{Duration, sleep};

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
    if args.iter().any(|arg| arg == "--tui") {
        if let Err(err) = oqqwall_tui::run_cli(&args) {
            eprintln!("tui: {err}");
            std::process::exit(1);
        }
        return;
    }

    println!("系统已启动");
    let app_config = match load_app_config_with_auto_oobe() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
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
    let (engine, handle) = Engine::new(core_config, &data_dir).expect("failed to init engine");
    debug_log!("engine created: data_dir={}", data_dir);
    let _status = status::spawn_status_logger(&handle);
    let _telemetry =
        telemetry::spawn_submission_telemetry(&handle, &app_config.telemetry, &data_dir);
    debug_log!("status logger spawned");
    web_api::spawn_web_api(&handle, &app_config);
    webview::spawn_webview(&handle, &app_config);
    debug_log!(
        "web services init: api_enabled={} api_port={} webview_enabled={} webview_host={} webview_port={}",
        app_config.web_api_enabled,
        app_config.web_api_port,
        app_config.webview_enabled,
        app_config.webview_host,
        app_config.webview_port
    );
    if let Err(err) = spawn_napcat_drivers(&handle, &app_config) {
        eprintln!("启动失败: {}", err);
        std::process::exit(1);
    }
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

fn load_app_config_with_auto_oobe() -> Result<AppConfig, String> {
    let config_path = config::resolve_config_path();
    let has_config = config::config_exists(&config_path)?;
    if !has_config {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            return Err(format!(
                "未找到配置文件 '{}'，且当前无交互终端，无法自动执行 OOBE。请手动运行: OQQWall_RUST oobe --config {}",
                config_path, config_path
            ));
        }
        println!("未找到配置文件 '{}'，正在进入 OOBE 初始化...", config_path);
        let oobe_args = vec![
            "oobe".to_string(),
            "--config".to_string(),
            config_path.clone(),
        ];
        oobe::run(&oobe_args)?;
    }
    AppConfig::load()
}
