# OQQWall_RUST OOBE（从 0 到 首次成功跑通）

> 目标：让你在 30 分钟内完成一次“投稿 → 审核 → 发送”的最小闭环。
> 说明：本文按**当前仓库实现**编写；部分能力仍是 stub（见第 0 节）。

---

## 0. 当前实现范围（先对齐预期）

- 已实现：NapCat OneBot WS 收消息、审核群指令解析、审核流程、发送队列与调度骨架。
- 已实现：Qzone sender 逻辑（真实网络请求），但需要 NapCat cookies 且受风控影响。
- 当前是 stub：渲染驱动只回写事件，不产出真实 SVG/PNG 文件。
- 审核群消息目前仅发送一行 `#<review_code> post <post_id>` 作为占位。

---

## 1. 前置准备（5 分钟）

- 准备一个能运行 NapCat 的 QQ 账号，并开启 OneBot WS（默认 `ws://127.0.0.1:8081`）。
- 记录 OneBot token（NapCat 配置里的 access_token）。
- 准备一个“审核群”，拿到群号（将写入 `mangroupid`）。
- 机器可访问外网（Qzone 发送需要；仅跑审核链路可不要求）。

---

## 2. 最小可运行配置（10 分钟）

推荐使用内置 OOBE 生成配置：

```bash
cargo run -p OQQWall_RUST -- oobe
```

也可以手动在项目根目录创建 `config.json`：

```json
{
  "common": {
    "process_waittime_sec": 20,
    "render_png": false,
    "tz_offset_minutes": 480
  },
  "groups": {
    "default": {
      "mangroupid": "123456",
      "napcat_ws_url": "ws://127.0.0.1:8081",
      "napcat_access_token": "admin",
      "mainqqid": "1234567890"
    }
  }
}
```

配置说明（仅列 OOBE 必需项）：

- `groups.<id>.napcat_ws_url`：该账号组对应的 NapCat OneBot WS 地址。
- `groups.<id>.napcat_access_token`：与该组 NapCat 侧 token 一致。
- `common.process_waittime_sec`：消息聚合窗口（秒）。
- `groups.<id>.mangroupid`：审核群 ID（为空则不会发送审核消息）。
- `groups.<id>.mainqqid`：主账号 QQ 号（用于发送账户选择）。
- `tz_offset_minutes`：本地时区偏移分钟数（中国大陆建议 `480`）。

---

## 3. 启动（3 分钟）

先确保 NapCat 已启动并能连上 OneBot WS，然后在项目根目录运行：

```bash
cargo run -p OQQWall_RUST
```

如果你的配置不在默认路径：

```bash
OQQWALL_CONFIG=/path/to/config.json cargo run -p OQQWall_RUST
```

---

## 4. 第一次投稿 → 审核 → 发送（10 分钟）

1) 在任意群或私聊发送一条消息（不要在审核群发送）。  
2) 等待 `process_waittime_sec` 秒后，审核群会收到一条消息：  
   `#<review_code> post <post_id>`  
3) 在审核群回复该消息或 @机器人 + 编号执行指令：
   - `是`：通过并进入发送队列
   - `否`：跳过/人工处理
   - `删`/`拒`：拒绝
   - `立即`：立即发送（高优先级）

> 说明：当前版本不推送渲染图，审核群只看到占位文本。

---

## 5. 验证清单（3 分钟）

- 审核群能收到 `#<review_code> post <post_id>`。
- 回复 `是` 后日志中应出现 SendPlan 或 SendStarted 相关事件。
- 若需要查看行为细节，可开启更详细日志级别（后续可补 tracing）。

---

## 6. 常见问题（排障速查）

- 没有任何消息进来：检查 `groups.<id>.napcat_ws_url` 与 token，确认 NapCat WS 在线。
- 审核群无消息：`mangroupid` 是否正确；确保投稿不是在审核群发出。
- 一直不成稿：`process_waittime_sec` 太大，先调到 10~20 秒。
- 发送失败：Qzone cookies 获取失败或风控；仅验证审核链路可忽略发送失败。

---

## 7. 下一步（推荐）

- 运行手册：`docs/runbook.md`（部署、systemd、升级/回滚）。
- 配置规范：`docs/config.md`（字段说明与兼容策略）。
- 指令手册：`docs/command.md`（完整审核/全局指令）。
