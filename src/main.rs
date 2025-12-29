mod connect;

use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Deserialize)]
struct Config {
    common: Value,
    #[serde(flatten)]
    groups: HashMap<String, Value>,
}

fn load_config() -> Result<Config, Box<dyn std::error::Error>> {
    let raw = std::fs::read_to_string("config.json")?;
    let config = serde_json::from_str(&raw)?;
    Ok(config)
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn read_string_or_number(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    group: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    match obj.get(field) {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(other) => {
            errors.push(format!(
                "error: group {group} {field} must be string or number, got {}",
                value_type_name(other)
            ));
            None
        }
    }
}

fn require_digits(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    group: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    match read_string_or_number(field, obj, group, errors) {
        None => {
            errors.push(format!("error: group {group} {field} is missing"));
            None
        }
        Some(value) => {
            if !is_digits(&value) {
                errors.push(format!(
                    "error: group {group} {field} must be numeric, got {value}"
                ));
                None
            } else {
                Some(value)
            }
        }
    }
}

fn require_string_non_empty(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    section: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    match obj.get(field) {
        None | Some(Value::Null) => {
            errors.push(format!("error: {section} {field} is missing"));
            None
        }
        Some(Value::String(value)) => {
            if value.is_empty() {
                errors.push(format!("error: {section} {field} must not be empty"));
                None
            } else {
                Some(value.clone())
            }
        }
        Some(Value::Number(value)) => {
            let value = value.to_string();
            if value.is_empty() {
                errors.push(format!("error: {section} {field} must not be empty"));
                None
            } else {
                Some(value)
            }
        }
        Some(other) => {
            errors.push(format!(
                "error: {section} {field} must be string, got {}",
                value_type_name(other)
            ));
            None
        }
    }
}

fn optional_digits(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    group: &str,
    errors: &mut Vec<String>,
) {
    if let Some(value) = read_string_or_number(field, obj, group, errors) {
        if !is_digits(&value) {
            errors.push(format!(
                "error: group {group} {field} must be numeric when set, got {value}"
            ));
        }
    }
}

fn require_boolish(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    section: &str,
    errors: &mut Vec<String>,
) -> Option<bool> {
    match obj.get(field) {
        None | Some(Value::Null) => {
            errors.push(format!("error: {section} {field} is missing"));
            None
        }
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(value)) => match value.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => {
                errors.push(format!(
                    "error: {section} {field} must be boolean-like true/false, got {value}"
                ));
                None
            }
        },
        Some(Value::Number(value)) => {
            if let Some(int) = value.as_i64() {
                match int {
                    0 => Some(false),
                    1 => Some(true),
                    _ => {
                        errors.push(format!(
                            "error: {section} {field} must be boolean-like 0/1, got {int}"
                        ));
                        None
                    }
                }
            } else {
                errors.push(format!(
                    "error: {section} {field} must be boolean-like number, got {}",
                    value
                ));
                None
            }
        }
        Some(other) => {
            errors.push(format!(
                "error: {section} {field} must be boolean-like, got {}",
                value_type_name(other)
            ));
            None
        }
    }
}

fn read_string_array(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    group: &str,
    errors: &mut Vec<String>,
) -> Option<Vec<String>> {
    match obj.get(field) {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let mut values = Vec::new();
            for item in items {
                match item {
                    Value::String(value) => values.push(value.clone()),
                    Value::Number(value) => values.push(value.to_string()),
                    Value::Null => continue,
                    other => {
                        errors.push(format!(
                            "error: group {group} {field} contains {}, expected string or number",
                            value_type_name(other)
                        ));
                    }
                }
            }
            Some(values)
        }
        Some(other) => {
            errors.push(format!(
                "error: group {group} {field} must be array, got {}",
                value_type_name(other)
            ));
            None
        }
    }
}

fn read_string_array_strict(
    field: &str,
    obj: &serde_json::Map<String, Value>,
    group: &str,
    errors: &mut Vec<String>,
) -> Option<Vec<String>> {
    match obj.get(field) {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let mut values = Vec::new();
            for item in items {
                match item {
                    Value::String(value) => values.push(value.clone()),
                    Value::Null => continue,
                    other => {
                        errors.push(format!(
                            "error: group {group} {field} contains {}, expected string",
                            value_type_name(other)
                        ));
                    }
                }
            }
            Some(values)
        }
        Some(other) => {
            errors.push(format!(
                "error: group {group} {field} must be array, got {}",
                value_type_name(other)
            ));
            None
        }
    }
}

fn is_time_hhmm(value: &str) -> bool {
    let mut parts = value.split(':');
    let Some(hour_str) = parts.next() else {
        return false;
    };
    let Some(min_str) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    if hour_str.is_empty() || hour_str.len() > 2 || !hour_str.chars().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    if min_str.len() != 2 || !min_str.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let Ok(hour) = hour_str.parse::<u32>() else {
        return false;
    };
    let Ok(minute) = min_str.parse::<u32>() else {
        return false;
    };
    hour <= 23 && minute <= 59
}

fn validate_groups(groups: &HashMap<String, Value>) -> Result<(), ()> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut mainqqid_seen = HashSet::new();
    let mut minorqqid_seen = HashSet::new();
    let mut http_ports_seen = HashSet::new();

    for (name, group_value) in groups {
        if name.trim().is_empty() {
            errors.push("error: empty group name detected".to_string());
            continue;
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            errors.push(format!(
                "error: group {name} contains invalid characters; only letters, digits, and underscore are allowed"
            ));
            continue;
        }
        let Some(group_obj) = group_value.as_object() else {
            errors.push(format!(
                "error: group {name} must be an object, got {}",
                value_type_name(group_value)
            ));
            continue;
        };

        let _mangroupid = require_digits("mangroupid", group_obj, name, &mut errors);
        let mainqqid = require_digits("mainqqid", group_obj, name, &mut errors);
        let mainqq_http_port = require_digits("mainqq_http_port", group_obj, name, &mut errors);

        if let Some(value) = mainqqid {
            if !mainqqid_seen.insert(value.clone()) {
                errors.push(format!(
                    "error: group {name} mainqqid {value} duplicated across groups"
                ));
            }
        }
        if let Some(value) = mainqq_http_port {
            if !http_ports_seen.insert(value.clone()) {
                errors.push(format!(
                    "error: group {name} mainqq_http_port {value} duplicated across groups"
                ));
            }
        }

        let minorqqids = read_string_array("minorqqid", group_obj, name, &mut errors);
        let minorqq_ports =
            read_string_array("minorqq_http_port", group_obj, name, &mut errors);

        if minorqqids.as_ref().map_or(true, |values| values.is_empty()) {
            warnings.push(format!("warning: group {name} minorqqid is empty"));
        }
        if minorqq_ports.as_ref().map_or(true, |values| values.is_empty()) {
            warnings.push(format!(
                "warning: group {name} minorqq_http_port is empty"
            ));
        }

        if let Some(values) = minorqqids.as_ref() {
            for value in values {
                if !is_digits(value) {
                    errors.push(format!(
                        "error: group {name} minorqqid contains non-numeric value {value}"
                    ));
                    continue;
                }
                if !minorqqid_seen.insert(value.clone()) {
                    errors.push(format!(
                        "error: group {name} minorqqid {value} duplicated across groups"
                    ));
                }
            }
        }

        if let Some(values) = minorqq_ports.as_ref() {
            for value in values {
                if !is_digits(value) {
                    errors.push(format!(
                        "error: group {name} minorqq_http_port contains non-numeric value {value}"
                    ));
                    continue;
                }
                if !http_ports_seen.insert(value.clone()) {
                    errors.push(format!(
                        "error: group {name} minorqq_http_port {value} duplicated across groups"
                    ));
                }
            }
        }

        let minorqq_count = minorqqids.as_ref().map_or(0, |values| values.len());
        let minorqq_port_count = minorqq_ports.as_ref().map_or(0, |values| values.len());
        if minorqq_count != minorqq_port_count {
            errors.push(format!(
                "error: group {name} minorqqid count {minorqq_count} does not match minorqq_http_port count {minorqq_port_count}"
            ));
        }

        optional_digits("max_post_stack", group_obj, name, &mut errors);
        optional_digits("max_image_number_one_post", group_obj, name, &mut errors);

        if let Some(value) = group_obj.get("friend_add_message") {
            if !value.is_null() && !value.is_string() {
                errors.push(format!(
                    "error: group {name} friend_add_message must be string or null, got {}",
                    value_type_name(value)
                ));
            }
        }
        if let Some(value) = group_obj.get("watermark_text") {
            if !value.is_null() && !value.is_string() {
                errors.push(format!(
                    "error: group {name} watermark_text must be string or null, got {}",
                    value_type_name(value)
                ));
            }
        }

        if let Some(schedule) =
            read_string_array_strict("send_schedule", group_obj, name, &mut errors)
        {
            for value in schedule {
                if !value.is_empty() && !is_time_hhmm(&value) {
                    errors.push(format!(
                        "error: group {name} send_schedule has invalid time {value}"
                    ));
                }
            }
        }

        if let Some(value) = group_obj.get("quick_replies") {
            if value.is_null() {
                continue;
            }
            let Some(map) = value.as_object() else {
                errors.push(format!(
                    "error: group {name} quick_replies must be object, got {}",
                    value_type_name(value)
                ));
                continue;
            };
            let audit_commands = [
                "是", "否", "匿", "等", "删", "拒", "立即", "刷新", "重渲染", "扩列审查",
                "评论", "回复", "展示", "拉黑", "消息全选",
            ];
            for (key, val) in map {
                if key.is_empty() {
                    continue;
                }
                let Some(text) = val.as_str() else {
                    errors.push(format!(
                        "error: group {name} quick_replies key {key} must map to string"
                    ));
                    continue;
                };
                if audit_commands.contains(&key.as_str()) {
                    errors.push(format!(
                        "error: group {name} quick_replies key {key} conflicts with audit command"
                    ));
                }
                if text.is_empty() {
                    errors.push(format!(
                        "error: group {name} quick_replies key {key} has empty value"
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        eprintln!("config validation failed:");
        for msg in errors {
            eprintln!("{msg}");
        }
        return Err(());
    }

    if !warnings.is_empty() {
        eprintln!("config validation warnings:");
        for msg in warnings {
            eprintln!("{msg}");
        }
    }

    Ok(())
}

fn validate_common(common: &Value) -> Result<(), ()> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let Some(obj) = common.as_object() else {
        eprintln!("error: common section must be an object, got {}", value_type_name(common));
        return Err(());
    };

    require_string_non_empty("napcat_access_token", obj, "common", &mut errors);
    require_boolish("manage_napcat_internal", obj, "common", &mut errors);
    require_boolish("renewcookies_use_napcat", obj, "common", &mut errors);
    require_digits("max_attempts_qzone_autologin", obj, "common", &mut errors);
    require_boolish("force_chromium_no_sandbox", obj, "common", &mut errors);
    require_boolish("at_unprived_sender", obj, "common", &mut errors);
    require_digits("friend_request_window_sec", obj, "common", &mut errors);
    require_boolish("use_web_review", obj, "common", &mut errors);
    require_digits("web_review_port", obj, "common", &mut errors);

    if let Some(token) = obj.get("napcat_access_token").and_then(|v| v.as_str()) {
        if token.eq_ignore_ascii_case("auto") {
            warnings.push("warning: common napcat_access_token is set to auto".to_string());
        }
    }

    if !errors.is_empty() {
        eprintln!("common validation failed:");
        for msg in errors {
            eprintln!("{msg}");
        }
        return Err(());
    }

    if !warnings.is_empty() {
        eprintln!("common validation warnings:");
        for msg in warnings {
            eprintln!("{msg}");
        }
    }

    Ok(())
}

fn main() {
    let config = match load_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("failed to load config.json: {err}");
            return;
        }
    };

    if validate_common(&config.common).is_err() {
        return;
    }

    if config.groups.is_empty() {
        eprintln!("config loaded but no group sections were found");
        return;
    }

    if validate_groups(&config.groups).is_err() {
        return;
    }

    println!("config loaded with {} groups", config.groups.len());
}
