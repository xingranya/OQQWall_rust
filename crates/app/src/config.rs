use std::env;
use std::fs;

use oqqwall_rust_core::{CoreConfig, GroupConfig};
use oqqwall_rust_drivers::napcat::NapCatConfig;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub napcat: NapCatConfig,
    pub default_group_id: String,
    pub audit_group_id: Option<String>,
    pub process_waittime_ms: i64,
    pub render_png: bool,
    pub tz_offset_minutes: i32,
}

impl AppConfig {
    pub fn load() -> Result<Self, String> {
        let path = env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string());
        let data = fs::read_to_string(&path)
            .map_err(|err| format!("failed to read config {}: {}", path, err))?;
        let root: Value =
            serde_json::from_str(&data).map_err(|err| format!("invalid config json: {}", err))?;
        Self::from_value(&root)
    }

    pub fn build_core_config(&self) -> CoreConfig {
        let mut core = CoreConfig::default();
        core.render_png = self.render_png;
        core.default_process_waittime_ms = self.process_waittime_ms;
        core.groups.insert(
            self.default_group_id.clone(),
            GroupConfig {
                group_id: self.default_group_id.clone(),
                process_waittime_ms: Some(self.process_waittime_ms),
                ..Default::default()
            },
        );
        core
    }

    fn from_value(root: &Value) -> Result<Self, String> {
        let (common, groups) = split_config(root)?;
        let default_group_id = resolve_default_group_id(&common, &groups)
            .unwrap_or_else(|| "default".to_string());
        let group_value = groups
            .get(&default_group_id)
            .or_else(|| groups.values().next())
            .cloned()
            .unwrap_or(Value::Null);
        let audit_group_id = parse_string(group_value.get("mangroupid"));

        let ws_url = env_override(
            "OQQWALL_NAPCAT_WS_URL",
            parse_string(common.get("napcat_ws_url")),
        )
        .unwrap_or_else(|| "ws://127.0.0.1:8081".to_string());
        let access_token = env_override(
            "OQQWALL_NAPCAT_TOKEN",
            parse_string(common.get("napcat_access_token")),
        );

        let process_waittime_ms = env::var("OQQWALL_PROCESS_WAITTIME_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .or_else(|| parse_duration_ms(common.get("process_waittime_ms")))
            .or_else(|| {
                parse_duration_ms(common.get("process_waittime_sec"))
                    .map(|v| v.saturating_mul(1000))
            })
            .or_else(|| {
                parse_duration_ms(common.get("process_waittime"))
                    .map(|v| v.saturating_mul(1000))
            })
            .unwrap_or(20_000);

        let render_png = parse_bool(common.get("render_png")).unwrap_or(false);
        let tz_offset_minutes = parse_i64(common.get("tz_offset_minutes"))
            .unwrap_or(0) as i32;
        Ok(Self {
            napcat: NapCatConfig {
                ws_url,
                access_token,
            },
            default_group_id,
            audit_group_id,
            process_waittime_ms,
            render_png,
            tz_offset_minutes,
        })
    }
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

fn resolve_default_group_id(common: &Value, groups: &std::collections::HashMap<String, Value>) -> Option<String> {
    if let Some(value) = parse_string(common.get("default_group_id")) {
        return Some(value);
    }
    let mut keys: Vec<String> = groups.keys().cloned().collect();
    keys.sort();
    keys.into_iter().next()
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

fn parse_duration_ms(value: Option<&Value>) -> Option<i64> {
    parse_i64(value)
}

fn env_override(key: &str, fallback: Option<String>) -> Option<String> {
    env::var(key).ok().or(fallback)
}
