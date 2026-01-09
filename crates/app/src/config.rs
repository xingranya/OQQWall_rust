use std::env;
use std::fs;
use std::collections::HashMap;

use oqqwall_rust_core::{CoreConfig, GroupConfig};
use oqqwall_rust_drivers::napcat::NapCatConfig;
use serde_json::Value;

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

#[derive(Debug, Clone)]
pub struct AppGroupConfig {
    pub group_id: String,
    pub audit_group_id: Option<String>,
    pub napcat: NapCatConfig,
    pub friend_request_window_sec: u32,
    pub friend_add_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub groups: Vec<AppGroupConfig>,
    pub tz_offset_minutes: i32,
    pub fallback_napcat: Option<NapCatConfig>,
    pub max_cache_mb: u64,
    pub at_unprived_sender: bool,
    core_config: CoreConfig,
    #[cfg(debug_assertions)]
    pub dev_config: DevConfig,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Default)]
pub struct DevConfig {
    pub use_virt_qzone: bool,
}

impl AppConfig {
    pub fn load() -> Result<Self, String> {
        let path = env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string());
        debug_log!("loading config path={}", path);
        let data = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read config {}: {}", path, err))?;
        debug_log!("config bytes={}", data.len());
        let root: Value =
            serde_json::from_str(&data).map_err(|err| format!("invalid config json: {}", err))?;
        Self::from_value(&root)
    }

    pub fn build_core_config(&self) -> CoreConfig {
        self.core_config.clone()
    }

    fn from_value(root: &Value) -> Result<Self, String> {
        let (common, groups) = split_config(root)?;
        let default_process_waittime_ms = env::var("OQQWALL_PROCESS_WAITTIME_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .or_else(|| {
                parse_duration_ms(common.get("process_waittime_sec"))
                    .map(|v| v.saturating_mul(1000))
            })
            .unwrap_or(20_000);

        let tz_offset_minutes = parse_i64(common.get("tz_offset_minutes"))
            .unwrap_or(0) as i32;
        let max_cache_mb = env::var("OQQWALL_MAX_CACHE_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| parse_u64(common.get("max_cache_mb")))
            .unwrap_or(256);
        let at_unprived_sender = parse_bool(common.get("at_unprived_sender")).unwrap_or(false);
        let default_friend_request_window_sec =
            parse_u32(common.get("friend_request_window_sec")).unwrap_or(300);
        let default_friend_add_message = parse_string(common.get("friend_add_message"));
        debug_log!(
            "config parsed: tz_offset_minutes={} default_process_waittime_ms={} max_cache_mb={} at_unprived_sender={}",
            tz_offset_minutes,
            default_process_waittime_ms,
            max_cache_mb,
            at_unprived_sender
        );
        let core_config = build_core_config(&common, &groups, default_process_waittime_ms);
        let fallback_napcat = parse_napcat_config_optional(&common);

        let mut group_configs = Vec::new();
        for (group_id, group_value) in &groups {
            let audit_group_id = parse_string(group_value.get("mangroupid"));
            let napcat = parse_napcat_config(&common, Some(group_value))
                .map_err(|err| format!("group {}: {}", group_id, err))?;
            let friend_request_window_sec =
                parse_u32(group_value.get("friend_request_window_sec"))
                    .unwrap_or(default_friend_request_window_sec);
            let friend_add_message = parse_string(group_value.get("friend_add_message"))
                .or_else(|| default_friend_add_message.clone());
            let _napcat_ws_log = ws_url_for_log(&napcat.ws_url);
            debug_log!(
                "config group: group_id={} audit_group_id={:?} napcat_ws_url={} napcat_token_present={}",
                group_id,
                audit_group_id,
                _napcat_ws_log,
                napcat.access_token.is_some()
            );
            group_configs.push(AppGroupConfig {
                group_id: group_id.clone(),
                audit_group_id,
                napcat,
                friend_request_window_sec,
                friend_add_message,
            });
        }
        if group_configs.is_empty() {
            let Some(napcat) = fallback_napcat.clone() else {
                return Err("missing groups and napcat_ws_url".to_string());
            };
            group_configs.push(AppGroupConfig {
                group_id: "default".to_string(),
                audit_group_id: None,
                napcat,
                friend_request_window_sec: default_friend_request_window_sec,
                friend_add_message: default_friend_add_message,
            });
        }
        #[cfg(debug_assertions)]
        let dev_config = load_dev_config()?;
        Ok(Self {
            groups: group_configs,
            tz_offset_minutes,
            fallback_napcat,
            max_cache_mb,
            at_unprived_sender,
            core_config,
            #[cfg(debug_assertions)]
            dev_config,
        })
    }
}

#[cfg(debug_assertions)]
fn load_dev_config() -> Result<DevConfig, String> {
    let path = "devconfig.json";
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            debug_log!("dev config missing: {}", path);
            return Ok(DevConfig::default());
        }
        Err(err) => return Err(format!("failed to read dev config {}: {}", path, err)),
    };
    let root: Value =
        serde_json::from_str(&data).map_err(|err| format!("invalid dev config json: {}", err))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "dev config must be a json object".to_string())?;
    let use_virt_qzone = parse_bool(obj.get("use-virt-qzone")).unwrap_or(false);
    debug_log!("dev config parsed: use_virt_qzone={}", use_virt_qzone);
    Ok(DevConfig { use_virt_qzone })
}

fn split_config(root: &Value) -> Result<(Value, std::collections::HashMap<String, Value>), String> {
    let obj = root
        .as_object()
        .ok_or_else(|| "config must be a json object".to_string())?;
    let common = obj.get("common").cloned().unwrap_or(Value::Null);
    if let Some(groups) = obj.get("groups").and_then(|v| v.as_object()) {
        let map = groups
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Ok((common, map));
    }
    let mut map = std::collections::HashMap::new();
    for (key, value) in obj {
        if key == "common" || key == "schema_version" {
            continue;
        }
        map.insert(key.clone(), value.clone());
    }
    Ok((common, map))
}

fn parse_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        Value::Number(n) => Some(n.as_i64().unwrap_or(0) != 0),
        _ => None,
    }
}

fn parse_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}

fn parse_usize(value: Option<&Value>) -> Option<usize> {
    parse_i64(value).and_then(|v| if v >= 0 { Some(v as usize) } else { None })
}

fn parse_u32(value: Option<&Value>) -> Option<u32> {
    parse_i64(value).and_then(|v| if v >= 0 { u32::try_from(v).ok() } else { None })
}

fn parse_u64(value: Option<&Value>) -> Option<u64> {
    parse_i64(value).and_then(|v| if v >= 0 { u64::try_from(v).ok() } else { None })
}

fn parse_duration_ms(value: Option<&Value>) -> Option<i64> {
    parse_i64(value)
}

fn env_override(key: &str, fallback: Option<String>) -> Option<String> {
    env::var(key).ok().or(fallback)
}

fn build_core_config(
    common: &Value,
    groups: &HashMap<String, Value>,
    default_process_waittime_ms: i64,
) -> CoreConfig {
    let mut core = CoreConfig::default();
    core.default_process_waittime_ms = default_process_waittime_ms;
    core.default_min_interval_ms =
        parse_duration_ms(common.get("min_interval_ms")).unwrap_or(0);
    core.default_max_queue = parse_usize(common.get("max_queue")).unwrap_or(0);
    core.default_max_images_per_post =
        parse_usize(common.get("max_image_number_one_post")).unwrap_or(30);
    core.default_send_timeout_ms =
        parse_duration_ms(common.get("send_timeout_ms")).unwrap_or(300_000);
    core.default_send_max_attempts = parse_u32(common.get("send_max_attempts")).unwrap_or(3);

    for (group_id, value) in groups {
        let process_waittime_ms = parse_duration_ms(value.get("process_waittime_sec"))
            .map(|v| v.saturating_mul(1000));

        let min_interval_ms = parse_duration_ms(value.get("min_interval_ms"));
        let max_queue = parse_usize(value.get("max_post_stack"));
        let max_images_per_post = parse_usize(value.get("max_image_number_one_post"));
        let send_timeout_ms = parse_duration_ms(value.get("send_timeout_ms"));
        let send_max_attempts = parse_u32(value.get("send_max_attempts"));
        let send_schedule_minutes = parse_schedule_minutes(value.get("send_schedule"));
        let accounts = parse_accounts(value);

        core.groups.insert(
            group_id.clone(),
            GroupConfig {
                group_id: group_id.clone(),
                process_waittime_ms,
                send_windows: Vec::new(),
                min_interval_ms,
                max_queue,
                max_images_per_post,
                send_schedule_minutes,
                accounts,
                send_timeout_ms,
                send_max_attempts,
            },
        );
    }

    if core.groups.is_empty() {
        let default_group_id = "default".to_string();
        core.groups.insert(
            default_group_id.clone(),
            GroupConfig {
                group_id: default_group_id,
                process_waittime_ms: Some(default_process_waittime_ms),
                send_timeout_ms: None,
                send_max_attempts: None,
                ..Default::default()
            },
        );
    }

    core
}

fn parse_accounts(value: &Value) -> Vec<String> {
    let accounts = parse_string_list(value.get("accounts"));
    if !accounts.is_empty() {
        return accounts;
    }

    let mut out = Vec::new();
    if let Some(main) = parse_string(value.get("mainqqid")) {
        out.push(main);
    }
    out.extend(parse_string_list(value.get("minorqqid")));
    out
}

fn parse_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| parse_string(Some(v)))
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect(),
        Value::Number(n) => vec![n.to_string()],
        _ => Vec::new(),
    }
}

fn parse_schedule_minutes(value: Option<&Value>) -> Vec<u16> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items.iter().filter_map(parse_schedule_item).collect(),
        Value::String(s) => s
            .split(',')
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .filter_map(parse_schedule_str)
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_schedule_item(value: &Value) -> Option<u16> {
    match value {
        Value::String(s) => parse_schedule_str(s),
        Value::Number(n) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        _ => None,
    }
}

fn parse_schedule_str(value: &str) -> Option<u16> {
    let mut parts = value.split(':');
    let hour = parts.next()?.trim().parse::<u16>().ok()?;
    let minute = parts.next()?.trim().parse::<u16>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour.saturating_mul(60).saturating_add(minute))
}

fn parse_napcat_config(common: &Value, group: Option<&Value>) -> Result<NapCatConfig, String> {
    let ws_url = resolve_napcat_ws_url(common, group)
        .ok_or_else(|| "missing napcat_ws_url".to_string())?;
    let access_token = resolve_napcat_token(common, group);

    let _ws_log = ws_url_for_log(&ws_url);
    debug_log!(
        "napcat config resolved: ws_url={} token_present={}",
        _ws_log,
        access_token.is_some()
    );
    Ok(NapCatConfig {
        ws_url,
        access_token,
    })
}

fn parse_napcat_config_optional(common: &Value) -> Option<NapCatConfig> {
    let ws_url = resolve_napcat_ws_url(common, None)?;
    let access_token = resolve_napcat_token(common, None);
    let _ws_log = ws_url_for_log(&ws_url);
    debug_log!(
        "napcat config resolved: ws_url={} token_present={}",
        _ws_log,
        access_token.is_some()
    );
    Some(NapCatConfig {
        ws_url,
        access_token,
    })
}

fn resolve_napcat_ws_url(common: &Value, group: Option<&Value>) -> Option<String> {
    let group_ws = group.and_then(|v| parse_string(v.get("napcat_ws_url")));
    let common_ws = parse_string(common.get("napcat_ws_url"));
    env_override("OQQWALL_NAPCAT_WS_URL", group_ws.or(common_ws))
}

fn resolve_napcat_token(common: &Value, group: Option<&Value>) -> Option<String> {
    let group_token = group.and_then(|v| parse_string(v.get("napcat_access_token")));
    let common_token = parse_string(common.get("napcat_access_token"));
    env_override("OQQWALL_NAPCAT_TOKEN", group_token.or(common_token))
}

#[cfg(debug_assertions)]
fn ws_url_for_log(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

#[cfg(not(debug_assertions))]
fn ws_url_for_log(_url: &str) -> &str {
    "<redacted>"
}
