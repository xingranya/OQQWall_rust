use std::fs::{OpenOptions, create_dir_all};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const LOG_ENV_VAR: &str = "OQQWALL_DEBUG_LOG";
const DATA_DIR_ENV_VAR: &str = "OQQWALL_DATA_DIR";
const DEFAULT_LOG_REL_PATH: &str = "logs/debug.log";

static LOG_FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

pub fn log(args: std::fmt::Arguments) {
    let message = format!("{}\n", args);
    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(message.as_bytes());
    let _ = stderr.flush();

    let Some(lock) = LOG_FILE.get_or_init(init_log_file).as_ref() else {
        return;
    };
    if let Ok(mut file) = lock.lock() {
        let _ = file.write_all(message.as_bytes());
        let _ = file.flush();
    }
}

fn init_log_file() -> Option<Mutex<std::fs::File>> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        if let Err(err) = create_dir_all(parent) {
            eprintln!("debug log: create dir failed {}: {}", parent.display(), err);
            return None;
        }
    }
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => Some(Mutex::new(file)),
        Err(err) => {
            eprintln!("debug log: open failed {}: {}", path.display(), err);
            None
        }
    }
}

fn log_path() -> PathBuf {
    if let Ok(path) = std::env::var(LOG_ENV_VAR) {
        return PathBuf::from(path);
    }
    let data_dir = std::env::var(DATA_DIR_ENV_VAR).unwrap_or_else(|_| "data".to_string());
    PathBuf::from(data_dir).join(DEFAULT_LOG_REL_PATH)
}
