# config.md — OQQWall_RUST 配置规则与加载/传递设计（JSON 优先）

> 本文面向：开发、运维、测试。  
> 目标：**配置不硬编码**、以 **JSON 文件**为权威来源；同时兼容原版 OQQWall 的配置语义与字段（原版拆成 `oqqwall.config` + `AcountGroupcfg.json`，并在多处脚本/服务端读取）。  
>
> 配置示例可直接使用文末的 JSON 模板（common + 单组），或从现有部署的 `oqqwall.config` / `AcountGroupcfg.json` 导入。

---

## 1. 原版 OQQWall 的配置来源与语义（作为兼容依据）

原版主要有两类配置源：

1) **全局配置（KV 文件）**：`oqqwall.config`  
- `serv.py` 读取 `oqqwall.config`（key=value + #注释），并要求必须有 `napcat_access_token`，且允许环境变量 `NAPCAT_ACCESS_TOKEN` 兜底  
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
- CLI：`oqqwallrs run --config ./config.json`
- 环境变量可覆盖：`OQQWALL_CONFIG=...`

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

## 3. 字段命名规范与“别名”兼容策略（非常重要）

原版很多 key 用 `-`（例如 `force_chromium_no-sandbox`、`http-serv-port`），你当前 JSON 样例用 snake_case（例如 `force_chromium_no_sandbox`）。

**Rust 版规范：配置 JSON 内推荐统一 snake_case**，并通过 `alias` 兼容旧 key：

* `http_serv_port` 兼容 `http-serv-port`（原版 `serv.py` 最后读取 `http-serv-port` 作为端口键）
* `force_chromium_no_sandbox` 兼容 `force_chromium_no-sandbox`（原版 `preprocess.sh` 读取的是带 `-` 的 key）
* `process_waittime_sec` 兼容 `process_waittime`（原版键名；冲突处理见下）

实现建议（Rust / serde）：

* 使用 `#[serde(alias="http-serv-port")]`、`#[serde(alias="force_chromium_no-sandbox")]`、`#[serde(alias="process_waittime")]`
* 对布尔/数字做“宽松解析”：允许 `"false"`/`false`、`"3"`/`3`（你的样例里大量数值与 bool 是字符串）

---

## 4. common（全局配置）字段说明

> 原版 TUI 给出全局配置键的提示集合（TOOLTIPS）以及固定顺序（ORDER）
> Rust 版可以“按这个集合为主”，并允许未来扩展字段（未知字段保留在 `extra`）。

### 4.1 字段表

| JSON Key (snake_case)        | 兼容别名                         |     类型 |                默认 | 原版语义/参考                                               |
| ---------------------------- | ---------------------------- | -----: | ----------------: | ----------------------------------------------------- |
| napcat_access_token          | napcat_access_token          | string | **必填**（也可 env 覆盖） | `serv.py` 必须配置，否则报错；也支持 env `NAPCAT_ACCESS_TOKEN` 兜底  |
| manage_napcat_internal       | manage_napcat_internal       |   bool |             false | 是否由系统内部管理 NapCat/QQ（原版有同名配置提示）                        |
| renewcookies_use_napcat      | renewcookies_use_napcat      |   bool |              true | 续 cookies 逻辑使用 NapCat 版本/非 NapCat 版本（原版提示）            |
| render_png | render_png | bool | false | 是否同时渲染 PNG；默认只产出 SVG（腾讯 QQ/空间接口可直接接受 SVG），开启后审核群可直接收到 PNG 图 |
| max_attempts_qzone_autologin | max_attempts_qzone_autologin |    u32 |                 3 | sendcontrol 默认 3 次并校验数字                               |
| at_unprived_sender           | at_unprived_sender           |   bool |             false | 通过时是否 @ 未公开空间的投稿人（sendcontrol 读取此 key）                |
| friend_request_window_sec    | friend_request_window_sec    |    u32 |               300 | 好友请求/私聊抑制窗口（原版 TUI 提示）                                |
| use_web_review               | use_web_review               |   bool |             false | 是否启用网页审核面板（原版提示）                                      |
| web_review_port              | web_review_port              |    u16 |             10923 | 网页审核监听端口（原版提示）                                        |
| http_serv_port               | http-serv-port               |    u16 |              8000 | 原版 `serv.py` 最终用 `http-serv-port` 决定 HTTP 端口          |
| process_waittime_sec         | process_waittime             |    u32 |                20 | 原版 `preprocess.sh` 读取 `process_waittime`（秒）           |
| force_chromium_no_sandbox    | force_chromium_no-sandbox    |   bool |             false | 原版根据此 key 决定 Chrome 是否加 `--no-sandbox`                |

关于 `process_waittime_sec` / `process_waittime`：

* 若两者同时存在且值不一致：启动时报错退出（避免误配）
* 若仅存在旧键：打印一次 warning，并在 `EffectiveConfig` 中归一化为 `_sec`

> 目前你说 AI 不做，那么 `apikey/text_model/vision_model/...` 这些可以留在 schema 中但运行时忽略；原版仍在 TUI 中列出这些键 。

### 4.2 环境变量覆盖优先级（推荐）

* `NAPCAT_ACCESS_TOKEN` > `config.json.common.napcat_access_token`
  理由：原版 `serv.py` 允许 `os.getenv('NAPCAT_ACCESS_TOKEN')` 兜底 。

---

## 5. groups（账号组配置）字段说明

> 原版 TUI 列出了组配置键顺序（GROUP_CONFIG_ORDER），覆盖了主要字段：mangroupid/mainqqid/端口/阈值/send_schedule/quick_replies/admins 。

### 5.1 字段表（每个 group 对象）

| JSON Key                  |                         类型 | 默认/规则            | 原版行为/参考                                                         |
| ------------------------- | -------------------------: | ---------------- | --------------------------------------------------------------- |
| mangroupid                |                     string | 必填               | 用于识别/管理群（`serv.py` 把它加入受管群集合）                                   |
| mainqqid                  |                     string | 必填               | 主账号 QQ 号，用于端口映射、组归属等                                            |
| mainqq_http_port          |                 string/u16 | 必填               | 主账号对应 NapCat/OneBot HTTP 端口（原版多处取此字段）                           |
| minorqqid                 |              array[string] | 可空               | 副账号 QQ 列表，注意长度可与端口数组不一致（原版 TUI 明确“按较短长度对齐”）                     |
| minorqq_http_port         |          array[string/u16] | 可空               | 副账号端口数组，同上                                                      |
| max_post_stack            |                        int | 默认 1；只允许正整数（1 表示单条直接发送，>1 启用暂存堆栈） | sendcontrol 对此字段做默认值与数字校验                                       |
| max_image_number_one_post |                        int | 默认 30；只允许正整数     | sendcontrol 同样校验并默认                                             |
| individual_image_in_posts |                       bool | 默认 true          | preprocess 缺省为 true，决定是否把用户原图也拷贝到 prepost（组策略）                  |
| send_schedule             |             array["HH:MM"] | 默认空（不启用定时 flush） | sendcontrol scheduler 从该字段读出 HH:MM 列表并按分钟触发 flush；同一时间点当日只触发一次  |
| watermark_text            |                     string | 默认 ""            | 原版用于渲染/展示（Rust 可保留用于 SVG 主题）                                    |
| friend_add_message        |                     string | 默认 ""            | 原版用于自动通过好友申请后发送文本（你样例包含）                                        |
| quick_replies             |     object{string->string} | 默认 {}            | 原版用于快捷回复，并在 processsend 做“快捷回复指令名冲突检测”                          |
| admins                    | array[{username,password}] | 默认 []            | 原版用于 web_review 管理员（你样例包含）                                      |

### 5.2 send_schedule 的语义（必须写清楚）

* `send_schedule` 是一组 **每日 HH:MM** 时间点（例如 `"15:05"`、`"23:55"`）
* sendcontrol 的 scheduler 每分钟 tick，一旦当前 `nowHM == HH:MM` 则触发 `flush_staged_posts`，并创建当日 markfile，保证**同日同时间只触发一次**。

Rust 版落地建议：

* scheduler 逻辑放到 `decide_on_tick()`（纯函数）里：当 `nowHM` 命中组 schedule 且当日未触发 → emit `GroupFlushRequested(group, now)`
* driver 执行 flush 后 emit `GroupFlushed / GroupFlushFailed`，并写“当日已触发”到 state（事件化），替代脚本中的 markfile。

---

## 6. 配置读取、校验、归一化（Rust 实现建议）

### 6.1 两阶段配置模型

1. `ConfigRaw`：serde 直接反序列化 JSON（宽松类型：string/bool/number 都能收）
2. `EffectiveConfig`：归一化后的强类型配置（bool/u32/u16、时间解析、端口解析、派生结构）

**归一化要做的事情：**

* 解析 bool：`"true"/"false"` 与 `true/false` 都接受
* 解析 int：`"3"` 与 `3` 都接受（你的样例是字符串）
* 解析端口：字符串/数字 → u16，范围校验 1..65535
* 解析 `send_schedule`：`HH:MM` 校验并转换为 `minutes_of_day`（0..1439）
* 对 `minorqqid` 与 `minorqq_http_port`：按较短长度对齐（原版 TUI 明确此规则）
* 应用默认值（参考原版 defaults：sendcontrol 的默认 max_attempts/max_post_stack/max_image_number 等）

### 6.2 校验失败策略（建议）

* **启动时**：关键字段缺失（如 napcat_access_token、mangroupid、mainqqid、端口）→ 直接报错退出
* **热更新时**：新配置解析失败 → 保留旧 `EffectiveConfig`，并发出告警/日志

---

## 7. 配置“读取与传递”设计（不硬编码、可热更新）

### 7.1 组件：ConfigManager（shell 层）

职责：

* 读取 JSON 文件
* 归一化 + 校验 → `EffectiveConfig`
* 提供 `ConfigHandle`：`ArcSwap<EffectiveConfig>`（或 `Arc<RwLock<..>>`，但 ArcSwap 更偏函数式快照）
* 可选：监控文件变更（notify crate）或支持 `SIGHUP` 触发 reload

### 7.2 与事件系统的集成（推荐）

* 引擎启动时：把 `EffectiveConfig` 注入到 Engine（不进 reducer）
* 每次 reload 成功后：append 一个 `ConfigApplied { config_version, config_blob }` 事件

  * `config_blob` 可选：把原始 JSON bytes 存入 BlobStore，便于事后审计/重放（与事件溯源理念一致）
* decider 纯函数签名建议变为：

  * `decide(state, cmd, cfg: &EffectiveConfig) -> Vec<Event>`
  * `reduce(state, event) -> state'`（cfg 不进 reducer）

> 这样：配置既“不硬编码”，又不会把大 JSON 混进 StateView（避免回放膨胀），同时仍能用事件记录“某时刻应用了什么配置版本”。

### 7.3 drivers 如何拿到配置

* drivers 通过 `ConfigHandle.load()` 获取最新 `Arc<EffectiveConfig>` 快照
* 对“必须一致的请求”：driver 在处理 `XxxRequested` 事件时，应使用事件内的参数为准（例如 retry_at / not_before），而不是用当前配置重新算——保持可重放一致性。

---

## 8. JSON 示例（推荐模板）

你上传的样例 `config.json` 已经符合“common + group”的思路，建议稍微增强为：

```json
{
  "schema_version": 1,
  "common": {
    "napcat_access_token": "REDACTED",
    "manage_napcat_internal": false,
    "renewcookies_use_napcat": true,
    "max_attempts_qzone_autologin": 3,
    "force_chromium_no_sandbox": false,
    "at_unprived_sender": true,
    "friend_request_window_sec": 300,
    "use_web_review": true,
    "web_review_port": 10923,
    "process_waittime_sec": 20,
    "http_serv_port": 8000
  },
  "groups": {
    "MethGroup": {
      "mangroupid": "993802974",
      "mainqqid": "3995477265",
      "mainqq_http_port": 3000,
      "minorqqid": [],
      "minorqq_http_port": [],
      "max_post_stack": 3,
      "max_image_number_one_post": 9,
      "individual_image_in_posts": false,
      "send_schedule": ["15:05", "23:55"],
      "watermark_text": "",
      "friend_add_message": "您的好友申请已通过，请阅读校园墙空间置顶后再投稿（系统自动发送请勿回复）",
      "quick_replies": {
        "格式错误": "您的投稿格式有误，请重新发送"
      },
      "admins": [
        { "username": "3391146750", "password": "admin" }
      ]
    }
  }
}
```

---

## 9. 兼容/迁移（可选，但强烈建议）

为了平滑迁移原版部署，建议提供命令：

* `oqqwallrs config import --oqqwall-config ./oqqwall.config --group-config ./AcountGroupcfg.json -o ./config.json`

导入规则：

* 读取 KV（原版 `read_config` 语义：去掉 `#` 注释，按 `=` 分割）
* 读取 `AcountGroupcfg.json` 作为 groups
* 处理 key alias（`http-serv-port`→`http_serv_port`、`force_chromium_no-sandbox`→`force_chromium_no_sandbox`）
* 输出统一 JSON

---

## 10. 开发落地 Checklist（让实现不走样）

* [ ] serde 结构：Raw + Effective 两层
* [ ] 宽松解析：bool/int/string 兼容（样例里大量是 string）
* [ ] schedule 解析：支持 `HH:MM`，并在 EffectiveConfig 中转成 minutes_of_day
* [ ] minor id/port 对齐：按 min(len(ids),len(ports))（原版 TUI 规则）
* [ ] env 覆盖：NAPCAT_ACCESS_TOKEN 优先（原版行为）
* [ ] config 变更：ConfigApplied 事件 +（可选）config_blob 入 BlobStore
* [ ] core 不直接读文件、不直接读 env（只接受 `&EffectiveConfig`）

---

## 11. 附：原版字段集合（便于对照）

原版 TUI 明确列出了一批全局 key 的说明（tooltip），并列出组 key 的顺序（包含 send_schedule、quick_replies、admins 等）。Rust 版可以把这些作为“兼容字段白名单”的基础。
