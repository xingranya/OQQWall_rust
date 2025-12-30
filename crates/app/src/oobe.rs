use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde_json::{json, Map, Value};

pub fn run(args: &[String]) -> Result<(), String> {
    let mut config_path = env::var("OQQWALL_CONFIG").unwrap_or_else(|_| "config.json".to_string());
    let mut force = false;

    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "oobe" | "--oobe" => {}
            "--config" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "missing value for --config".to_string())?;
                config_path = value.to_string();
            }
            "--force" => {
                force = true;
            }
            "-h" | "--help" => {
                print_oobe_help();
                return Ok(());
            }
            other => {
                return Err(format!("unknown oobe argument: {}", other));
            }
        }
    }

    if Path::new(&config_path).exists() && !force {
        let overwrite = prompt_bool(
            &format!("配置文件 '{}' 已存在，是否覆盖？", config_path),
            false,
        )?;
        if !overwrite {
            println!("已取消，未写入配置。");
            return Ok(());
        }
    }

    println!("开始配置 OQQWall_RUST（按回车使用默认值）");
    let group_id = prompt_with_default("逻辑组名（group id）", "default")?;
    let audit_group_id = prompt_required("审核群 ID（mangroupid）")?;
    let mainqqid = prompt_required("主账号 QQ 号（mainqqid）")?;
    let minorqqid = prompt_csv_strings("副账号 QQ 号列表（逗号分隔，可留空）")?;

    let napcat_ws_url = prompt_with_default("本组 NapCat WS 地址", "ws://127.0.0.1:3001")?;
    let access_token =
        prompt_optional("本组 NapCat access_token（留空则运行时用环境变量）")?;
    let process_waittime_sec = prompt_u64("聚合窗口秒数（process_waittime_sec）", 20)?;
    let render_png = prompt_bool("是否渲染 PNG（render_png）", false)?;
    let tz_offset_minutes = prompt_i64("时区偏移分钟数（中国大陆=480）", 480)?;

    let max_post_stack = prompt_u64("最大暂存条数（max_post_stack）", 1)?;
    let max_image_number_one_post = prompt_u64("单条最大图片数（max_image_number_one_post）", 30)?;
    let individual_image_in_posts = prompt_bool("是否保留原图（individual_image_in_posts）", true)?;
    let send_schedule = prompt_schedule("定时发送（HH:MM 逗号分隔，可留空）")?;

    let mut common = Map::new();
    common.insert(
        "process_waittime_sec".to_string(),
        Value::Number(process_waittime_sec.into()),
    );
    common.insert("render_png".to_string(), Value::Bool(render_png));
    common.insert(
        "tz_offset_minutes".to_string(),
        Value::Number(tz_offset_minutes.into()),
    );

    let mut group = Map::new();
    group.insert(
        "mangroupid".to_string(),
        Value::String(audit_group_id),
    );
    group.insert("napcat_ws_url".to_string(), Value::String(napcat_ws_url));
    if let Some(token) = access_token {
        group.insert("napcat_access_token".to_string(), Value::String(token));
    }
    group.insert("mainqqid".to_string(), Value::String(mainqqid));
    group.insert(
        "minorqqid".to_string(),
        Value::Array(
            minorqqid
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    group.insert(
        "max_post_stack".to_string(),
        Value::Number(max_post_stack.into()),
    );
    group.insert(
        "max_image_number_one_post".to_string(),
        Value::Number(max_image_number_one_post.into()),
    );
    group.insert(
        "individual_image_in_posts".to_string(),
        Value::Bool(individual_image_in_posts),
    );
    if !send_schedule.is_empty() {
        group.insert(
            "send_schedule".to_string(),
            Value::Array(send_schedule.into_iter().map(Value::String).collect()),
        );
    }

    let mut groups = Map::new();
    groups.insert(group_id, Value::Object(group));

    let root = json!({
        "schema_version": 1,
        "common": Value::Object(common),
        "groups": Value::Object(groups),
    });

    let mut output =
        serde_json::to_string_pretty(&root).map_err(|err| format!("json error: {}", err))?;
    output.push('\n');
    fs::write(&config_path, output)
        .map_err(|err| format!("failed to write {}: {}", config_path, err))?;

    println!("已写入配置：'{}'.", config_path);
    Ok(())
}

fn print_oobe_help() {
    println!("OQQWall_RUST oobe");
    println!();
    println!("用法:");
    println!("  OQQWall_RUST oobe [--config <path>] [--force]");
    println!();
    println!("选项:");
    println!("  --config <path>  配置路径（默认：$OQQWALL_CONFIG 或 ./config.json）");
    println!("  --force          不提示直接覆盖已有配置");
}

fn prompt_line(prompt: &str) -> Result<String, String> {
    print!("{}", prompt);
    io::stdout()
        .flush()
        .map_err(|err| format!("stdout error: {}", err))?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| format!("stdin error: {}", err))?;
    Ok(input.trim().to_string())
}

fn prompt_with_default(label: &str, default: &str) -> Result<String, String> {
    let input = prompt_line(&format!("{} [{}]: ", label, default))?;
    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input)
    }
}

fn prompt_required(label: &str) -> Result<String, String> {
    loop {
        let input = prompt_line(&format!("{}: ", label))?;
        if !input.is_empty() {
            return Ok(input);
        }
        println!("该项不能为空。");
    }
}

fn prompt_optional(label: &str) -> Result<Option<String>, String> {
    let input = prompt_line(&format!("{}: ", label))?;
    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input))
    }
}

fn prompt_bool(label: &str, default: bool) -> Result<bool, String> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        let input = prompt_line(&format!("{} [{}]: ", label, hint))?;
        if input.is_empty() {
            return Ok(default);
        }
        let value = input.to_ascii_lowercase();
        match value.as_str() {
            "y" | "yes" | "true" | "1" => return Ok(true),
            "n" | "no" | "false" | "0" => return Ok(false),
            _ => println!("请输入 yes 或 no。"),
        }
    }
}

fn prompt_u64(label: &str, default: u64) -> Result<u64, String> {
    loop {
        let input = prompt_line(&format!("{} [{}]: ", label, default))?;
        if input.is_empty() {
            return Ok(default);
        }
        if let Ok(value) = input.parse::<u64>() {
            return Ok(value);
        }
        println!("请输入有效数字。");
    }
}

fn prompt_i64(label: &str, default: i64) -> Result<i64, String> {
    loop {
        let input = prompt_line(&format!("{} [{}]: ", label, default))?;
        if input.is_empty() {
            return Ok(default);
        }
        if let Ok(value) = input.parse::<i64>() {
            return Ok(value);
        }
        println!("请输入有效数字。");
    }
}

fn prompt_csv_strings(label: &str) -> Result<Vec<String>, String> {
    let input = prompt_line(&format!("{}: ", label))?;
    Ok(parse_csv(&input))
}

fn prompt_schedule(label: &str) -> Result<Vec<String>, String> {
    loop {
        let input = prompt_line(&format!("{}: ", label))?;
        if input.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut ok = true;
        for part in parse_csv(&input) {
            match normalize_schedule_item(&part) {
                Some(item) => out.push(item),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Ok(out);
        }
        println!("格式错误，示例：15:05,23:55");
    }
}

fn parse_csv(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn normalize_schedule_item(value: &str) -> Option<String> {
    let mut parts = value.split(':');
    let hour = parts.next()?.trim().parse::<u32>().ok()?;
    let minute = parts.next()?.trim().parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(format!("{:02}:{:02}", hour, minute))
}
