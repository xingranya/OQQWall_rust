# HTTP API v1

本文档描述当前 Rust 版本已实现的对外 HTTP 审核接口。

## 1. 启用与配置

启用条件：
- `common.web_api.enabled = true`
- `common.web_api.root_token`（或环境变量 `OQQWALL_API_TOKEN`）存在且长度 >= 32

监听地址：
- `0.0.0.0:${common.web_api.port}`
- 默认端口：`10923`

兼容读取（启动时自动迁移）：
- `common.use_web_review` -> `common.web_api.enabled`
- `common.web_review_port` -> `common.web_api.port`
- `common.api_token` -> `common.web_api.root_token`

## 2. 鉴权模型

- 登录接口：`POST /v1/auth/login`
- 其他接口：`Authorization: Bearer <session_id>`
- Session 为临时会话（当前默认 12 小时）

权限枚举：
- `review.read`
- `review.write`
- `send.execute`
- `blacklist.read`
- `blacklist.write`
- `session.manage`
- `token.manage`

## 3. 统一错误返回

```json
{
  "error": {
    "code": "PERMISSION_DENIED",
    "message": "permission denied",
    "request_id": "req_xxx"
  }
}
```

常见状态码：
- `200`：成功
- `204`：成功且无响应体
- `400`：参数错误
- `401`：未认证或 session 失效
- `403`：权限不足
- `404`：资源不存在
- `422`：当前模型不支持该请求语义
- `503`：引擎命令通道不可用

## 4. 接口定义

### 4.1 登录

`POST /v1/auth/login`

请求：
```json
{
  "token": "root_or_sub_token"
}
```

响应：
```json
{
  "session_id": "32hex...",
  "expires_at": 1730000000,
  "permissions": ["review.read", "review.write"]
}
```

### 4.2 登出

`POST /v1/auth/logout`

请求头：`Authorization: Bearer <session_id>`

响应：`204 No Content`

### 4.3 强制下线 Session

`POST /v1/auth/sessions/{session_id}/revoke`

权限：`session.manage`

响应：`204 No Content`

### 4.4 创建子 Token

`POST /v1/auth/tokens`

权限：`token.manage`

请求：
```json
{
  "permissions": ["review.read", "review.write"],
  "expire_at": 1730000000
}
```

响应：
```json
{
  "token": "32hex...",
  "token_id": "tok_2",
  "expire_at": 1730000000
}
```

### 4.5 稿件列表

`GET /v1/posts?stage=review_pending&cursor=0&limit=50`

权限：`review.read`

参数：
- `stage` 可选：
  - `drafted`
  - `render_requested`
  - `rendered`
  - `review_pending`
  - `reviewed`
  - `scheduled`
  - `sending`
  - `sent`
  - `rejected`
  - `skipped`
  - `manual`
  - `failed`
- `cursor` 可选，默认 `0`
- `limit` 可选，默认 `50`，范围 `1..200`

响应：
```json
{
  "items": [
    {
      "post_id": "123",
      "review_id": "456",
      "group_id": "10001",
      "stage": "review_pending",
      "external_code": 1193,
      "internal_code": 102,
      "sender_id": "1050373508",
      "created_at_ms": 1730000000123,
      "last_error": null
    }
  ],
  "next_cursor": 1
}
```

### 4.6 稿件详情

`GET /v1/posts/{post_id}`

权限：`review.read`

响应（示例）：
```json
{
  "post_id": "123",
  "review_id": "456",
  "review_code": 102,
  "group_id": "10001",
  "stage": "review_pending",
  "external_code": 1193,
  "sender_id": "1050373508",
  "session_id": "999",
  "created_at_ms": 1730000000123,
  "is_anonymous": false,
  "is_safe": true,
  "blocks": [
    {"kind":"text","text":"正文"},
    {
      "kind":"attachment",
      "media_kind":"image",
      "reference_type":"blob_id",
      "reference":"777",
      "size_bytes": 12345
    }
  ],
  "render_png_blob_id": "888",
  "last_error": null
}
```

### 4.7 审核决策

`POST /v1/reviews/{review_id}/decision`

权限：`review.write`

请求：
```json
{
  "action": "approve|reject|delete|defer|skip|immediate|refresh|rerender|select_all|toggle_anonymous|expand_audit|show|comment|reply|blacklist|quick_reply|merge",
  "comment": "可选",
  "delay_ms": 60000,
  "text": "可选（comment/reply）",
  "quick_reply_key": "可选（quick_reply）",
  "target_review_code": 1234
}
```

说明：
- `blacklist` 时 `comment` 作为拉黑理由。
- `defer` 时可传 `delay_ms`。
- `comment/reply` 需要 `text`（兼容读取 `comment`）。
- `quick_reply` 需要 `quick_reply_key`。
- `merge` 需要 `target_review_code`。
- 支持幂等请求头：`Idempotency-Key: <key>`。

响应：
```json
{
  "review_id": "456",
  "status": "applied"
}
```

### 4.8 批量审核决策

`POST /v1/reviews/batch`

权限：`review.write`

请求：
```json
{
  "review_ids": ["456", "789"],
  "action": "approve"
}
```

响应：
```json
{
  "accepted": 2,
  "failed": []
}
```

### 4.9 Blob 读取

`GET /v1/blobs/{blob_id}`

权限：`review.read`

响应：二进制流（`Content-Type` 按文件扩展名推断）。

### 4.10 黑名单列表

`GET /v1/blacklist?group_id=10001&cursor=0&limit=50`

权限：`blacklist.read`

响应：
```json
{
  "items": [
    {
      "group_id": "10001",
      "sender_id": "1050373508",
      "reason": "广告"
    }
  ],
  "next_cursor": null
}
```

### 4.11 新增黑名单

`POST /v1/blacklist`

权限：`blacklist.write`

请求：
```json
{
  "group_id": "10001",
  "sender_id": "1050373508",
  "reason": "广告"
}
```

响应：`204 No Content`

### 4.12 删除黑名单

`DELETE /v1/blacklist/{group_id}/{sender_id}`

权限：`blacklist.write`

响应：`204 No Content`

### 4.13 触发发件

`POST /v1/posts/send`

权限：`send.execute`

请求：
```json
{
  "post_ids": ["123", "124"],
  "mode": "immediate|scheduled",
  "schedule_at": null
}
```

响应：
```json
{
  "accepted": 1,
  "failed": [
    {
      "post_id": "124",
      "reason": "post has no review_id"
    }
  ]
}
```

`schedule_at` 语义：
- 预期用途：当 `mode = scheduled` 时指定未来发送时间（Unix 秒/毫秒时间戳）。
- 当前实现状态：尚未接入调度链路，传入 `schedule_at` 会返回 `422`。

## 5. 兼容性说明

- `chooseall` 属于前端交互状态，不提供独立后端接口。
- 发送能力仍复用现有审核/调度事件链；接口是命令入口，不直接绕过引擎状态机。
