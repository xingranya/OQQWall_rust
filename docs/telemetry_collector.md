# telemetry_collector.md — 遥测采集服务端（独立进程）

> 目标：接收 OQQWall_RUST 上传的投稿训练样本，做幂等入库、对象落盘、样本管理和 Parquet 导出。

---

## 1. 架构与职责

`crates/telemetry-collector` 是独立二进制，不与主程序 `OQQWall_RUST` 共享进程。

- 上传入口：`POST /telemetry/v1/submission/batch`
- 存储：PostgreSQL（元数据）+ 本地对象目录（chat object 原文）
- 认证：Bearer Token + RBAC
- 管理：批次/样本查询、样本修订、token 管理、导出任务管理

---

## 2. 环境变量

- `COLLECTOR_HTTP_ADDR`：监听地址（默认 `0.0.0.0:10925`）
- `COLLECTOR_PG_DSN`：Postgres 连接串（必填）
- `COLLECTOR_OBJECT_DIR`：聊天对象目录（默认 `data/collector/objects`）
- `COLLECTOR_EXPORT_DIR`：导出目录（默认 `data/collector/exports`）
- `COLLECTOR_BOOTSTRAP_ROOT_TOKEN`：启动时写入/更新 `root` token（必填，>=16）
- `COLLECTOR_MAX_BODY_MB`：请求体上限（默认 10）
- `RUST_LOG`：日志级别

---

## 3. 上传协议与幂等

与主程序当前上传协议兼容：

- Header
  - `Authorization: Bearer <token>`
  - `Idempotency-Key: <key>`
- Body
  - `batch_id`
  - `schema_version`（当前只接受 `1`）
  - `chat_objects[]`
  - `samples[]`

幂等规则：

- 相同 `Idempotency-Key` + 相同 payload hash：返回 `200`，`duplicate=true`
- 相同 key + 不同 payload：返回 `409 IDEMPOTENCY_CONFLICT`
- 首次成功写入：返回 `201`

客户端仍按既有语义处理：只要收到 `2xx` 就视为整批 ACK。

---

## 4. 管理 API

需要 token 权限：

- `ingest.write`
- `batches.read`
- `samples.read`
- `samples.write`
- `exports.manage`
- `tokens.manage`

接口：

- `GET /telemetry/v1/healthz`
- `GET /telemetry/v1/batches`
- `GET /telemetry/v1/batches/{batch_id}`
- `GET /telemetry/v1/samples`
- `GET /telemetry/v1/samples/{sample_id}`
- `PATCH /telemetry/v1/samples/{sample_id}`
- `GET /telemetry/v1/chat_objects/{chat_record_hash}`
- `POST /telemetry/v1/exports`
- `GET /telemetry/v1/exports`
- `GET /telemetry/v1/exports/{job_id}`
- `GET /telemetry/v1/exports/{job_id}/manifest`
- `GET /telemetry/v1/exports/{job_id}/files/{name}`
- `POST /telemetry/v1/admin/tokens`
- `DELETE /telemetry/v1/admin/tokens/{token_id}`

---

## 5. 导出格式

导出任务默认 `parquet`，按目录分区：

```text
<export_root>/<job_id>/
  manifest.json
  decision_date=YYYY-MM-DD/
    label=0/part-*.parquet
    label=1/part-*.parquet
```

`manifest.json` 包含：

- `job_id`
- `schema_version`
- `row_count`
- `filters`
- `files[]`（path + row_count + sha256）

---

## 6. Docker 运行

`Dockerfile.telemetry-collector` 采用“宿主机先编译、镜像只打包二进制”的方式。

### 6.0 先构建 Linux 兼容二进制

```bash
docker run --rm --network host \
  -v "$PWD:/work" -w /work \
  -v "$HOME/.cargo/registry:/root/.cargo/registry" \
  -v "$HOME/.cargo/git:/root/.cargo/git" \
  rust-glibc231:20.04-oqqwall \
  bash -lc 'CARGO_TARGET_DIR=/work/out-target cargo build --release -p oqqwall_telemetry_collector --bin telemetry-collector && cp /work/out-target/release/telemetry-collector /work/out/telemetry-collector'
```

### 6.1 使用 compose（推荐）

```bash
docker compose -f docker-compose.telemetry.yml up -d --build
```

服务：

- Postgres：`postgres:16-alpine`
- Collector：`telemetry-collector`（端口 `10925`）

### 6.2 健康检查

```bash
curl http://127.0.0.1:10925/telemetry/v1/healthz
```

### 6.3 与主程序对接

在 OQQWall_RUST 的 `config.json`：

- `common.telemetry.upload_enabled = true`
- `common.telemetry.upload_endpoint = "http://<collector-host>:10925/telemetry/v1/submission/batch"`
- `common.telemetry.upload_token = "<ingest token>"`

---

## 7. 数据保留

当前实现不做自动 TTL 清理（永久保留）。如需清理，请通过外部任务按业务策略执行。
