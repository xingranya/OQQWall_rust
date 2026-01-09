# OQQWall_RUST 开放 QQ 校园墙自动运营系统（Rust 单机版）

> OQQWall 的 Rust 重写/单机版实现：采用 Functional Core / Imperative Shell 架构，通过 NapCat/OneBot 接入 QQ 投稿、审核与发送 QQ 空间；当前全链路直接渲染 PNG。

## 简介
本系统用于“校园墙”日常运营：收稿 → 聚合成稿 → 渲染预览 → 审核群指令 → 排程发送 QQ 空间。  
Rust 版的目标是把原版脚本链路重构成可测试、可回放、可演进的事件驱动系统（并尽量保持原版的使用体验与指令语义）。

开始前请注意：NapCat/OneBot 属于 QQ 非官方接入方式，存在账号风控风险；请自行评估并在可控账号上使用。

# <div align=center>文档</div>
## <div align=center > [快速开始](OQQWall_rust.wiki/快速开始.md) | [全部文档](OQQWall_rust.wiki/Home.md)</div>

## 功能概览
- NapCat WS 收稿：群聊/私聊 → Ingress 事件
- `(chat_id, user_id)` 时间窗口聚合，超时自动成稿
- Draft 构建 + PNG 渲染
- 审核群发布 + 审核指令（是/否/删/等），兼容“回复审核消息执行”
- Qzone Sender：通过 NapCat `get_cookies` 拉取凭据发送空间（短暂缓存）
- 失败重试/退避（按错误类型）

## 路线图（Rust 版）
- 接入 `crates/infra` 的 journal/snapshot/blob 落盘，支持重启回放与快速恢复
- 补齐运维子命令（`doctor/replay/export` 等）与 Web 审核面板（可选）
- 渲染器补齐更多消息类型与排版细节（对齐原版效果）

## 目录结构
- `crates/app`：二进制入口与装配（配置、runtime、TUI/OOBE）
- `crates/core`：纯函数核心（Event/State/Reducer/Decider）
- `crates/drivers`：IO 驱动（NapCat WS、Qzone Sender、渲染）
- `crates/infra`：本地基础设施（journal/snapshot/blob，逐步接入）

## 快速开始（摘要）
1. 启动 NapCat，并开启 OneBot WS（例如：`ws://127.0.0.1:3001/ws`）。
2. 生成配置（可选）：`cargo run -p OQQWall_RUST -- oobe`
3. 运行：`cargo run -p OQQWall_RUST`

更完整的从 0 跑通流程见 `OQQWall_rust.wiki/快速开始.md`。

## 文档索引
- `OQQWall_rust.wiki/Home.md`：Wiki 主页（从此进入）
- `docs/oobe.md`：OOBE（从 0 到首次跑通）
- `docs/config.md`：配置规范与字段说明
- `docs/command.md`：群内指令与审核指令
- `docs/runbook.md`：部署/运维/排障手册
- `docs/engineering.md`：工程落地与架构约束
- `docs/dev_guide.md`：设计与演进路线

## 开源项目列表
本项目使用/参考了：Rust、Tokio、Serde、Skia（渲染）、NapCat/OneBot 生态等；并继承原版 OQQWall 的指令与交互习惯。
