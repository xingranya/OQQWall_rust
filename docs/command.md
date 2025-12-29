# OQQWall-rs 指令手册（docs/commands.md）

> 本文档用于开发/运维/管理员使用与对齐。  
> 设计目标：**尽可能兼容原版 OQQWall 的群内指令体系**，在 Rust 重构版中保留相同的“全局指令 / 审核指令”体验。  
> 参考来源：原版 `getmsgserv/command.sh` 的帮助文本与指令分派逻辑（全局指令与审核指令均在脚本中定义）:contentReference[oaicite:0]{index=0}

---
## 0. 审核请求发送

* #内部编号
* 发件zhe
预览内容分情况
若设定了只渲染svg：
* 消息概览：一行一条文本消息，多媒体显示为[图片]/[视频]等。合并转发消息合集需要展开，如 
 ```
 文本xxxxx
 [图片]
[合并转发消息]——
xxxxx
xxxxx
[图片]
xxxxx
——————
文本xxxxxx
```
若设定了渲染PNG
预览部分直接发送渲染后的PNG

* 随后是投稿中的图片和表情包
  
以上内容需要合并在一条消息中发送，如
```
#123 来自xxxxxx(qq号)
消息概览：
 文本xxxxx
 [图片]
[合并转发消息]——
xxxxx
xxxxx
[图片]
xxxxx
——————
文本xxxxxx
图片：
[一张真的图片]
[一张真的图片]
```

如果设定了渲染PNG
则为：
```
#123 来自xxxxxx(qq号)
[渲染出的图]
[用户发来的图]
[用户发来的图]

```

## 1. 指令触发方式

原版 OQQWall 支持两种常见触发方式：

1) **@机器人执行（推荐兼容）**  
- 全局指令语法：`@本账号/次要账号 指令`:contentReference[oaicite:1]{index=1}  
- 审核指令语法：`@本账号 内部编号 指令 [参数...]`:contentReference[oaicite:2]{index=2}  

2) **回复审核消息执行（推荐兼容）**  
- 审核指令也可“回复审核消息，只发指令”:contentReference[oaicite:3]{index=3}  

> Rust 版建议：两种都支持，并优先“回复审核消息”以减少输入内部编号的错误。

---

## 2. 核心概念

### 2.1 内部编号（tag）
原版脚本把“内部编号”作为第一参数 `object`，并将其当作数字分支处理：`case $object in [0-9]*) ...`:contentReference[oaicite:4]{index=4}  
同时要求对应目录存在（如 `./cache/prepost/$object`）才可执行:contentReference[oaicite:5]{index=5}。

> Rust 版建议：内部编号对应 `post_id` 的短码（如 6 位 `review_code`）与内部自增 `tag` 可以同时存在：  
> - 管理员交互：优先短码（更易输入）  
> - 内部存储/兼容：保留 tag 概念（便于与原逻辑/数据迁移对应）

### 2.2 指令分词（兼容性要点）
原版脚本用 `awk` 取前三段：`object`/`command`/`flag`:contentReference[oaicite:6]{index=6}。  
这意味着：**原版天然对“多空格、多词参数”支持较弱**（例如 `评论`/`回复` 的内容在原实现里更像单 token）。

> Rust 版建议（兼容+增强）：  
> - 仍然兼容 “前三段” 语义：`object command flag`  
> - 同时支持 “尾部参数整段”：将第 3 段及之后拼成 `args_text`（支持空格与引用）  
> - 解析优先级：  
>   1) 若是 `快捷回复 添加 指令名=内容`，优先按 `=` 切分  
>   2) 其它命令按 `object command [rest...]` 解析

---

## 3. 全局指令（任何时刻可用）

原版帮助文本声明：这些是“任何时刻@本账号调用的指令”，语法为 `@本账号/次要账号 指令`:contentReference[oaicite:7]{index=7}。

> 下表为 Rust 版建议“保持同名同义”，并在实现时对应到事件系统（例如 `OutboundReplyRequested`、`SystemCheckRequested` 等）。

| 指令 | 语法 | 原版语义/说明 | Rust 版实现建议 |
|---|---|---|---|
| 帮助 | `@机器人 帮助` | 输出完整帮助列表（全局+审核指令）:contentReference[oaicite:8]{index=8} | 直接回复 `commands.md` 的精简版 + 链接到完整文档 |
| 调出 | `@机器人 调出 <内部编号>` | 调出曾经接收到过的投稿，执行 `preprocess.sh <tag> randeronly`:contentReference[oaicite:9]{index=9}:contentReference[oaicite:10]{index=10} | 映射为：根据 tag/post_id 重新生成渲染产物并展示/发送预览 |
| 信息 | `@机器人 信息 <内部编号>` | 查询该编号的接收者、发送者、所属组、处理后 JSON 等:contentReference[oaicite:11]{index=11} | 映射为：输出 post 元信息（来源、账号组、状态、失败原因、产物 id） |
| 手动重新登录 | `@机器人 手动重新登录` | 扫码登录 QQ 空间:contentReference[oaicite:12]{index=12} | 触发 daemon/driver 进入“需要人工扫码”流程并提示操作 |
| 自动重新登录 | `@机器人 自动重新登录` | 尝试自动登录 QQ 空间:contentReference[oaicite:13]{index=13} | 触发自动刷新会话；失败则进入人工介入态 |
| 待处理 | `@机器人 待处理` | 列出当前等待处理投稿（按账号组过滤）:contentReference[oaicite:14]{index=14} | 输出待审核/待发送列表（按账号组） |
| 删除待处理 | `@机器人 删除待处理` | 清空待处理列表；原注释称“相当于对列表中的所有项目执行‘删’审核指令”:contentReference[oaicite:15]{index=15} | 需要谨慎：建议仅管理员可用；实现为批量归档/删除待审核项 |
| 删除暂存区 | `@机器人 删除暂存区` | 清空暂存区内容，并回滚外部编号:contentReference[oaicite:16]{index=16}:contentReference[oaicite:17]{index=17} | Rust 版若无“暂存区”概念，可映射为：清空发送队列（并记录审计日志） |
| 发送暂存区 | `@机器人 发送暂存区` | 将暂存区内容发送到 QQ 空间（通过 sendcontrol flush）:contentReference[oaicite:18]{index=18}:contentReference[oaicite:19]{index=19} | 映射为：立即触发 scheduler/sender flush（忽略 not_before，按顺序发送） |
| 列出拉黑 | `@机器人 列出拉黑` | 列出当前被拉黑账号列表:contentReference[oaicite:20]{index=20} | 输出黑名单（sender_id -> reason） |
| 取消拉黑 | `@机器人 取消拉黑 <senderid>` | 取消对某账号拉黑:contentReference[oaicite:21]{index=21} | 黑名单删除并确认 |
| 设定编号 | `@机器人 设定编号 <纯数字>` | 设定下一条说说外部编号:contentReference[oaicite:22]{index=22}:contentReference[oaicite:23]{index=23} | 若 Rust 版有“外部编号”，保留；否则可做为“显示编号前缀”配置项 |
| 快捷回复 | `@机器人 快捷回复` | 列出当前账号组快捷回复列表:contentReference[oaicite:24]{index=24} | 输出模板列表 |
| 快捷回复 添加 | `@机器人 快捷回复 添加 指令名=内容` | 添加快捷回复；且会检查不与审核指令冲突:contentReference[oaicite:25]{index=25} | 以配置事件写入（ConfigApplied/QuickReplyUpdated） |
| 快捷回复 删除 | `@机器人 快捷回复 删除 指令名` | 删除快捷回复:contentReference[oaicite:26]{index=26} | 同上 |
| 自检 | `@机器人 自检` | 系统自检（CPU/内存/硬盘/服务状态），必要时尝试重启服务:contentReference[oaicite:27]{index=27} | Rust 版输出 health summary；重启动作通过 daemon 实现 |
| 系统修复 | `@机器人 系统修复` | 重启除 serv.py 外服务并重建 UDS（强修复）:contentReference[oaicite:28]{index=28} | Rust 版：重启 drivers/清理 socket/重启 NapCat（谨慎开放权限） |

---

## 4. 审核指令（只在审核流程中使用）

原版帮助明确：审核指令语法为 `@本账号 内部编号 指令` 或 “回复审核消息 指令”:contentReference[oaicite:29]{index=29}。  
审核指令清单与说明如下（原版描述）：:contentReference[oaicite:30]{index=30}

> Rust 版注意：你当前阶段不做 AI，所以部分“刷新/重渲染/消息全选/匿”等会变成“纯渲染/纯重跑流水线”的含义。为了兼容体验，仍建议保留同名命令。

| 指令 | 语法（两种） | 原版语义/说明 | Rust 版建议实现 |
|---|---|---|---|
| 是 | `@机器人 <内部编号> 是` 或 回复 `是` | 发送，并给稿件发送者发送成功提示:contentReference[oaicite:31]{index=31} | 等价于“通过并入队发送”；成功后可私聊通知投稿人 |
| 否 | `… 否` | 机器跳过此条，人工处理（常用于分段/匿名失败或含视频）:contentReference[oaicite:32]{index=32} | Rust 版可映射为：标记 `ManualInterventionRequired` 或 `ReviewDelayed` |
| 匿 | `… 匿` | 切换匿名状态，处理后会再次询问指令:contentReference[oaicite:33]{index=33} | 当前无 AI：可改为“匿名开关”字段切换 + 重新渲染并重新发布审核 |
| 等 | `… 等` | 等待 180 秒后重新执行分段-渲染-审核:contentReference[oaicite:34]{index=34} | 映射为：`ReviewDelayed(now+180s)` + 到点重跑渲染/发布 |
| 删 | `… 删` | 此条不发送，也不用人工发送；外部编号+1:contentReference[oaicite:35]{index=35} | 映射为：归档/删除草稿（是否+1 外部编号视 Rust 版实现） |
| 拒 | `… 拒` | 拒绝稿件；给发送者发送被拒提示:contentReference[oaicite:36]{index=36} | 映射为：Rejected + 私聊通知 |
| 立即 | `… 立即` | 立刻发送暂存区全部投稿，并立即把当前投稿单发:contentReference[oaicite:37]{index=37} | 映射为：队列 flush + 当前 post 优先级提升到最高并立刻发送 |
| 刷新 | `… 刷新` | 重新进行“聊天记录->图片”:contentReference[oaicite:38]{index=38} | 无 AI：重跑渲染（从原始 ingress 重建 draft → 渲染） |
| 重渲染 | `… 重渲染` | 重做渲染但不重做 AI 分段（调试渲染）:contentReference[oaicite:39]{index=39} | 直接重新渲染（draft 不变） |
| 消息全选 | `… 消息全选` | 强制把本次投稿所有消息作为内容并重渲染:contentReference[oaicite:40]{index=40} | 映射为：draft builder 使用 “all messages” 模式重建 blocks |
| 扩列审查 | `… 扩列审查` | 扩列审核流程（抓等级/空间/名片/二维码等）:contentReference[oaicite:41]{index=41} | Rust 版可先保留命令但返回“未实现/已禁用”，或做插件化 |
| 评论 | `… 评论 <文本>` | 增加文本评论，处理后再次询问:contentReference[oaicite:42]{index=42} | 映射为：给 post 增加评论 block → 重新渲染 → 重新发布审核 |
| 回复 | `… 回复 <文本>` | 向投稿人发送一条信息:contentReference[oaicite:43]{index=43} | OneBot 私聊投稿人 |
| 展示 | `… 展示` | 展示稿件内容:contentReference[oaicite:44]{index=44} | 回复渲染链接/输出文本摘要 |
| 拉黑 | `… 拉黑 [理由]` | 不再接收来自此人的投稿:contentReference[oaicite:45]{index=45} | 将 sender_id 加入黑名单（可带理由） |
| 快捷回复指令 | `… <快捷指令名>` | 使用预设模板向投稿人发送消息:contentReference[oaicite:46]{index=46} | 根据账号组 quick_replies 模板渲染并私聊发送 |

---

## 5. 权限与作用域

原版脚本存在“按账号组权限判断”的逻辑（例如执行对象 tag 时会检查 tag 所属 ACgroup 是否等于当前 groupname，否者报权限错误）:contentReference[oaicite:47]{index=47}。  

明确两条权限规则：

1) **全局指令**：仅允许审核群内管理员执行  
2) **审核指令**：仅允许审核群内管理员执行，并且限定只能操作本账号组的 post（匹配 group）

---

## 6. 兼容性与增强建议（给开发用）

### 6.1 兼容原版两种输入形式
- 完整形式：`@机器人 <内部编号> <指令> [参数...]`:contentReference[oaicite:48]{index=48}  
- 回复形式：回复审核消息，仅输入 `<指令> [参数...]`:contentReference[oaicite:49]{index=49}  

### 6.2 Rust 版建议的解析规范（兼容+更好用）
- 先判断是否“回复审核消息”：若是，默认绑定到该审核项（无需内部编号）
- 若不是回复：支持两种格式  
  1) `<内部编号> <指令> [args...]`  
  2) `<指令> [args...]`（仅对全局指令）
- 参数规则：`args_text` 是剩余所有内容（支持空格），避免原版只取前三段的局限:contentReference[oaicite:50]{index=50}

---

## 7. 常见示例

### 7.1 全局
- `@机器人 帮助`
- `@机器人 待处理`
- `@机器人 调出 123`
- `@机器人 信息 123`
- `@机器人 自检`

### 7.2 审核（回复审核消息）
- 回复审核消息：`是`
- 回复审核消息：`拒 太敏感了`
- 回复审核消息：`评论 这条已核实`
- 回复审核消息：`回复 你好，这条需要补充信息`

### 7.3 审核（@机器人 + 内部编号）
- `@机器人 123 是`
- `@机器人 123 删`
- `@机器人 123 拉黑 广告刷屏`

---

## 8. 实现对照（开发备注）

- 原版帮助文本包含“全局指令”和“审核指令”的完整列表与语义描述:contentReference[oaicite:51]{index=51}  
- 原版对输入拆分 `object/command/flag` 的方式来自脚本的 `awk` 分词:contentReference[oaicite:52]{index=52}  
- 数字 object（内部编号）分支会进入针对 tag 的处理流程，并且有组权限检查:contentReference[oaicite:53]{index=53}  
- Rust 版如要 1:1 兼容，建议把“内部编号 tag”与“review_code（短码）”都保留，并实现互相映射。
