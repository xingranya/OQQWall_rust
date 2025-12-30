# OQQWall_RUST

OQQWall 的 Rust 单机版实现，采用 Functional Core / Imperative Shell 架构，用 NapCat/OneBot 接入 QQ 投稿、审核与发送 QQ 空间。当前默认全链路 SVG 渲染，按需开启 PNG。

## 功能概览
- NapCat WS 收稿：群聊/私聊 -> Ingress 事件
- `(chat_id, user_id)` 时间窗口聚合，超时自动成稿
- Draft 构建 + SVG 渲染（可选 PNG）
- 审核群发布 + 审核指令（是/否/删/等）
- Qzone Sender：通过 NapCat `get_cookies` 拉取凭据发送空间
- 失败重试/退避（按错误类型）

## 目录结构
- `crates/core`：纯函数核心（Event/State/Reducer/Decider）
- `crates/infra`：本地基础设施（journal/snapshot/blob，待接入）
- `crates/drivers`：IO 驱动（NapCat WS、Qzone Sender）
- `crates/app`：二进制入口与装配

## 快速开始
1. 启动 NapCat，并开启 OneBot WS（默认：`ws://127.0.0.1:8081`）。
2. 准备 `config.json`：

```json
{
  "common": {
    "napcat_ws_url": "ws://127.0.0.1:8081",
    "napcat_access_token": "admin",
    "process_waittime_sec": 20,
    "render_png": false,
    "default_group_id": "default"
  },
  "groups": {
    "default": {
      "mangroupid": "123456"
    }
  }
}
```

3. 运行：

```bash
cargo run -p OQQWall_RUST
```

可选环境变量：
- `OQQWALL_CONFIG`：配置文件路径（默认 `./config.json`）
- `OQQWALL_NAPCAT_WS_URL`：覆盖 `napcat_ws_url`
- `OQQWALL_NAPCAT_TOKEN`：覆盖 `napcat_access_token`
- `OQQWALL_PROCESS_WAITTIME_MS`：覆盖聚合窗口（毫秒）

## 配置要点
- `napcat_access_token` 必填，可用 `OQQWALL_NAPCAT_TOKEN` 代替落盘。
- `mangroupid` 为审核群 ID（必填）。
- Qzone cookies 不写配置，运行时通过 NapCat WS 获取并短暂缓存。
- `render_png=true` 时开启 PNG 渲染，否则全链路仅 SVG。
- `process_waittime_sec` 控制消息聚合时间窗口。

## 测试
```bash
cargo test
```

## 文档索引
- `docs/engineering.md`：工程落地与架构约束
- `docs/config.md`：配置规范与字段说明
- `docs/command.md`：群内指令与审核指令
- `docs/runbook.md`：部署/运维/排障手册
- `docs/dev_guide.md`：设计与演进路线

## 现状说明
- 引擎目前为内存态，journal/snapshot 的持久化尚未接入（计划见 `docs/engineering.md`）。
