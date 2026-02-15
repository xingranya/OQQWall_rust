# config.md — OQQWall_RUST 配置规则与加载/传递设计（JSON 优先）

> 本文面向：开发、运维、测试。  
> 目标：**配置不硬编码**、以 **JSON 文件**为权威来源；同时兼容原版 OQQWall 的配置语义与字段（原版拆成 `oqqwall.config` + `AcountGroupcfg.json`，并在多处脚本/服务端读取）。  
>
> 配置示例可直接使用文末的 JSON 模板（common + 单组），或从现有部署的 `oqqwall.config` / `AcountGroupcfg.json` 导入。

---

## 1. 原版 OQQWall 的配置来源与语义（作为兼容依据）

原版主要有两类配置源：

1) **全局配置（KV 文件）**：`oqqwall.config`  
- `serv.py` 读取 `oqqwall.config`（key=value + #注释），并要求必须有 `napcat_access_token`，原版允许 env 兜底；Rust 版改为每组配置 `napcat_access_token`，并支持 `OQQWALL_NAPCAT_TOKEN` 全局覆盖  
- `sendcontrol.sh` 也固定把全局配置文件名写为 `oqqwall.config`，并从中读取 `max_attempts_qzone_autologin`、`at_unprived_sender`，且给出默认值与校验逻辑

2) **账号组配置（JSON 文件）**：`AcountGroupcfg.json`  
- `serv.py` 会加载 `AcountGroupcfg.json`，建立 self_id→组名映射，并提取 `mangroupid` 作为受管群集合  
- `sendcontrol.sh` 会从 `AcountGroupcfg.json` 里按 receiver 找到组配置，并对 `max_post_stack`、`max_image_number_one_post` 做默认值/数字校验  
- `sendcontrol.sh` 的定时调度器会读取每个组的 `send_schedule`（HH:MM 列表），在对应分钟触发 `flush_staged_posts`，并保证“当日每个时间点只触发一次 + 同一分钟互斥锁”  
- `preprocess.sh` 会读取全局 `process_waittime` 与 `force_chromium_no-sandbox`，并从组配置取 `individual_image_in_posts`，且缺省为 true  

> Rust 版：我们不复刻原版的“散落在脚本里 grep/jq”读取方式，而是统一用一个 JSON 作为权威配置；但字段语义、默认值与校验尽量对齐上面这些行为。

---

## 2. OQQWall_RUST 推荐的统一 JSON 配置文件

### 2.1 文件名与位置
- 默认：`./config.json`
- 环境变量可覆盖：`OQQWALL_CONFIG=...`
  - 主程序启动时读取该路径（当前版本不解析 `--config` 参数）
  - OOBE/TUI 支持 `--config <path>` 写入/编辑指定配置

### 2.2 顶层结构（推荐规范）
推荐显式 `schema_version` 与 `groups`：

```json
{
  "schema_version": 1,
  "common": { },
  "groups": {
    "MethGroup": { }
  }
}
```

### 2.3 顶层结构（兼容简写）

为了兼容你当前提供的样例（`common` + 组名直接作为顶层键），允许：

```json
{
  "common": { },
  "MethGroup": { },
  "AnotherGroup": { }
}
```

规则：如果存在 `groups` 字段，则只读 `groups`；否则把除 `common`、`schema_version` 外的顶层对象都视为 group。

---

## 3. common（全局配置）字段说明

> 原版 TUI 给出全局配置键的提示集合（TOOLTIPS）以及固定顺序（ORDER）
> Rust 版可以“按这个集合为主”，并允许未来扩展字段（未知字段保留在 `extra`）。

### 3.1 字段表

| JSON Key (snake_case)        |     类型 |                默认 | 当前支持状态             | 原版语义/参考                                               |
| ---------------------------- | -----: | ----------------: | ------------------ | ----------------------------------------------------- |
| manage_napcat_internal       |   bool |             false | 不再支持               | 是否由系统内部管理 NapCat/QQ（原版有同名配置提示）                        |
| renewcookies_use_napcat      |   bool |              true | 未支持               | 续 cookies 逻辑使用 NapCat 版本/非 NapCat 版本（原版提示）            |
| max_attempts_qzone_autologin |    u32 |                 3 | 未支持               | sendcontrol 默认 3 次并校验数字                               |
| friend_request_window_sec    |    u32 |               300 | 已支持               | 好友请求/私聊抑制窗口（原版 TUI 提示）                                |
| web_api.enabled              |   bool |             false | 已支持               | 是否启用对外 HTTP 审核 API（替代旧 `use_web_review`）                            |
| web_api.port                 |    u16 |             10923 | 已支持               | HTTP API 监听端口（默认 `0.0.0.0:10923`，替代旧 `web_review_port`）              |
| web_api.root_token           | string |                "" | 已支持               | API root token（建议 32+ 位；可被环境变量覆盖，替代旧 `api_token`）               |
| webview.enabled              |   bool |             false | 已支持               | 是否启用内置 WebView 审核前端（账号密码登录）                                     |
| webview.host                 | string |         `0.0.0.0` | 已支持               | WebView 监听主机（可设为 `127.0.0.1` 仅本机访问）                                |
| webview.port                 |    u16 |             10924 | 已支持               | WebView 服务监听端口（默认 `0.0.0.0:10924`）                                      |
| webview.session_ttl_sec      |    i64 |             43200 | 已支持               | WebView 会话有效期（秒，默认 12h）                                                |
| napcat_base_url              | string |                "" | 已支持               | 作为默认 NapCat 反向 WS base url（推荐，优先级最高）                        |
| napcat_access_token          | string |                "" | 已支持               | 作为默认 NapCat token（可被 `OQQWALL_NAPCAT_TOKEN` 覆盖）             |
| tz_offset_minutes            |    i32 |                 0 | 已支持               | 时区偏移（分钟，用于 schedule/defer 计算）                           |
| min_interval_ms              |    u32 |                 0 | 已支持               | 发送最小间隔（毫秒）                                             |
| max_image_number_one_post    |    u32 |                30 | 已支持               | 单条最大图片数；超限会拆分发送，并触发暂存区 flush                       |
| send_timeout_ms              |    u32 |            300000 | 已支持               | 发送超时（毫秒）                                               |
| send_max_attempts            |    u32 |                 3 | 已支持               | 发送失败最大重试次数                                             |
| max_cache_mb                 |    u32 |               256 | 已支持               | 内存图片缓存上限（MB），超限时优先淘汰大文件缓存                          |
| process_waittime_sec         |    u32 |                20 | 已支持               | 原版 `preprocess.sh` 读取该值（秒）                           |

### 3.2 环境变量覆盖优先级（推荐）

* `OQQWALL_NAPCAT_TOKEN` > `groups.<id>.napcat_access_token`（全局覆盖所有组）
* `OQQWALL_NAPCAT_BASE_URL` > `groups.<id>.napcat_base_url`（全局覆盖所有组）
* `OQQWALL_API_TOKEN` > `common.web_api.root_token`（覆盖 HTTP API root token）

兼容迁移说明（启动时自动改写）：
* `common.use_web_review` -> `common.web_api.enabled`
* `common.web_review_port` -> `common.web_api.port`
* `common.api_token` / `common.token` -> `common.web_api.root_token`
* `groups.<id>.admins` -> `groups.<id>.webview_admins`

---

## 4. groups（账号组配置）字段说明

> 当前实现以 `accounts` 为账号列表来源；其中 `accounts[0]` 是主账号。兼容读取旧字段 `mainqqid/minorqqid` 与别名 `acount`，启动时会自动迁移并写回为 `accounts`。

### 4.1 字段表（每个 group 对象）

| JSON Key                  |                         类型 | 默认/规则            | 当前支持状态                 | 原版行为/参考                                                         |
| ------------------------- | -------------------------: | ---------------- | ---------------------- | --------------------------------------------------------------- |
| mangroupid                |                     string | 必填               | 已支持                   | 审核群 ID：审核指令/回复仅在该群处理；其他群消息会被忽略                           |
| napcat_base_url           |                     string | 必填               | 已支持                   | 本组 NapCat 反向 WS base url（推荐）                                  |
| napcat_access_token       |                     string | 必填（可 env 覆盖）   | 已支持                   | 本组 NapCat token；可用 `OQQWALL_NAPCAT_TOKEN` 覆盖                 |
| accounts                  |              array[string] | 必填（至少 1 个；首项为主账号） | 已支持                   | 账号列表，按顺序作为主号/替补优先级；审核相关群消息仅由当前有效主账号发送，主号离线按顺序替补 |
| max_post_stack            |                        int | 默认 1；只允许正整数（1 表示单条直接发送，>1 启用暂存堆栈） | 已支持                   | sendcontrol 对此字段做默认值与数字校验                                       |
| max_image_number_one_post |                        int | 默认 30；只允许正整数     | 已支持                   | 单条最大图片数；超限会拆分发送，并触发暂存区 flush                       |
| individual_image_in_posts |                       bool | 默认 true          | 已支持                   | true=发送渲染图+原图，false=仅发送渲染图                                   |
| at_unprived_sender           | at_unprived_sender           |   bool |             false | 已支持               | 发件时是否 @ 非匿名的投稿人（sendcontrol 读取此 key）                |
| send_schedule             |             array["HH:MM"] | 默认空（不启用定时 flush） | 已支持                   | sendcontrol scheduler 从该字段读出 HH:MM 列表并按分钟触发 flush；同一时间点当日只触发一次  |
| watermark_text            |                     string | 默认 ""            | 已支持                   | 用于渲染水印文本（空字符串不绘制水印）                                    |
| friend_add_message        |                     string | 默认 ""            | 已支持                   | 用于自动通过好友申请后发送文本（你样例包含）                                        |
| quick_replies             |     object{string->string} | 默认 {}            | 已支持                   | 用于快捷回复（键和值均为非空字符串，且键不能与审核指令冲突）              |
| webview_admins           | array[{username,password,role}] | 默认 []            | 已支持                   | WebView 组管理员；`role` 缺省为 `group_admin`，密码会归一化为 `sha256:` |

> 反向 WS 连接格式：NapCat 里填写 `ws://<host>/<base_path>/<QQ号>`，其中 `<base_path>` 来自 `napcat_base_url`（示例：`ws://127.0.0.1:3001/oqqwall/ws/456787654`）。

### 4.2 send_schedule 的语义（必须写清楚）

* `send_schedule` 是一组 **每日 HH:MM** 时间点（例如 `"15:05"`、`"23:55"`）
* sendcontrol 的 scheduler 每分钟 tick，一旦当前 `nowHM == HH:MM` 则触发 `flush_staged_posts`，并创建当日 markfile，保证**同日同时间只触发一次**。

Rust 版落地建议：

* scheduler 逻辑放到 `decide_on_tick()`（纯函数）里：当 `nowHM` 命中组 schedule 且当日未触发 → emit `GroupFlushRequested(group, now)`
* driver 执行 flush 后 emit `GroupFlushed / GroupFlushFailed`，并写“当日已触发”到 state（事件化），替代脚本中的 markfile。

---

## 5. 配置读取、校验、归一化（Rust 实现建议）

### 5.1 两阶段配置模型

1. `ConfigRaw`：serde 直接反序列化 JSON（宽松类型：string/bool/number 都能收）
2. `EffectiveConfig`：归一化后的强类型配置（bool/u32/u16、时间解析、端口解析、派生结构）

**归一化要做的事情：**

* 解析 bool：`"true"/"false"` 与 `true/false` 都接受
* 解析 int：`"3"` 与 `3` 都接受（你的样例是字符串）
* 解析 `send_schedule`：`HH:MM` 校验并转换为 `minutes_of_day`（0..1439）
* 应用默认值（参考原版 defaults：sendcontrol 的默认 max_attempts/max_post_stack/max_image_number 等）

### 5.2 校验失败策略（建议）

* **启动时**：关键字段缺失（如 napcat_base_url、napcat_access_token、mangroupid、accounts）→ 直接报错退出
* **热更新时**：新配置解析失败 → 保留旧 `EffectiveConfig`，并发出告警/日志

---

## 6. 配置“读取与传递”设计（不硬编码、可热更新）

### 6.1 组件：ConfigManager（shell 层）

职责：

* 读取 JSON 文件
* 归一化 + 校验 → `EffectiveConfig`
* 提供 `ConfigHandle`：`ArcSwap<EffectiveConfig>`（或 `Arc<RwLock<..>>`，但 ArcSwap 更偏函数式快照）
* 可选：监控文件变更（notify crate）或支持 `SIGHUP` 触发 reload

### 6.2 与事件系统的集成（推荐）

* 引擎启动时：把 `EffectiveConfig` 注入到 Engine（不进 reducer）
* 每次 reload 成功后：append 一个 `ConfigApplied { config_version, config_blob }` 事件

  * `config_blob` 可选：把原始 JSON bytes 存入 BlobStore，便于事后审计/重放（与事件溯源理念一致）
* decider 纯函数签名建议变为：

  * `decide(state, cmd, cfg: &EffectiveConfig) -> Vec<Event>`
  * `reduce(state, event) -> state'`（cfg 不进 reducer）

> 这样：配置既“不硬编码”，又不会把大 JSON 混进 StateView（避免回放膨胀），同时仍能用事件记录“某时刻应用了什么配置版本”。

### 6.3 drivers 如何拿到配置

* drivers 通过 `ConfigHandle.load()` 获取最新 `Arc<EffectiveConfig>` 快照
* 对“必须一致的请求”：driver 在处理 `XxxRequested` 事件时，应使用事件内的参数为准（例如 retry_at / not_before），而不是用当前配置重新算——保持可重放一致性。

---

## 7. JSON 示例（推荐模板）

你上传的样例 `config.json` 已经符合“common + group”的思路，建议稍微增强为：

```json
{
  "schema_version": 1,
  "common": {
    "manage_napcat_internal": false,
    "renewcookies_use_napcat": true,
    "max_attempts_qzone_autologin": 3,
    "force_chromium_no_sandbox": false,
    "at_unprived_sender": true,
    "friend_request_window_sec": 300,
    "web_api": {
      "enabled": true,
      "port": 10923,
      "root_token": "REDACTED"
    },
    "webview": {
      "enabled": true,
      "host": "127.0.0.1",
      "port": 10924,
      "session_ttl_sec": 43200
    },
    "process_waittime_sec": 20
  },
  "groups": {
    "MethGroup": {
      "mangroupid": "993802974",
      "napcat_base_url": "0.0.0.0:3001/oqqwall/ws",
      "napcat_access_token": "REDACTED",
      "accounts": ["3995477265"],
      "max_post_stack": 1,
      "max_image_number_one_post": 9,
      "send_schedule": ["15:05", "23:55"],
      "friend_add_message": "您的好友申请已通过，请阅读校园墙空间置顶后再投稿（系统自动发送请勿回复）",
      "webview_admins": [
        { "username": "3391146750", "password": "sha256:REDACTED", "role": "group_admin" }
      ]
    }
  },
  "webview_global_admins": [
    { "username": "root", "password": "sha256:REDACTED", "role": "global_admin" }
  ]
}
```

---

## 8. 兼容/迁移（可选，但强烈建议）

为了平滑迁移原版部署，建议提供命令：

* `OQQWall_RUST config import --oqqwall-config ./oqqwall.config --group-config ./AcountGroupcfg.json -o ./config.json`

导入规则：

* 读取 KV（原版 `read_config` 语义：去掉 `#` 注释，按 `=` 分割）
* 读取 `AcountGroupcfg.json` 作为 groups
* 输出统一 JSON

---

## 9. 开发落地 Checklist（让实现不走样）

* [ ] serde 结构：Raw + Effective 两层
* [ ] 宽松解析：bool/int/string 兼容（样例里大量是 string）
* [ ] schedule 解析：支持 `HH:MM`，并在 EffectiveConfig 中转成 minutes_of_day
* [ ] env 覆盖：OQQWALL_NAPCAT_TOKEN 优先（全组覆盖）
* [ ] config 变更：ConfigApplied 事件 +（可选）config_blob 入 BlobStore
* [ ] core 不直接读文件、不直接读 env（只接受 `&EffectiveConfig`）

---

## 10. 附：原版字段集合（便于对照）

原版 TUI 明确列出了一批全局 key 的说明（tooltip），并列出组 key 的顺序（包含 send_schedule、quick_replies、admins 等）。Rust 版可以把这些作为“兼容字段白名单”的基础。
