# 设计变更记录

> 历史记录保留当时的编号与措辞；现行编号约定见 `docs/architecture.md` 附录 A。

## 2026-10-31（第 94 轮，动态写阶段契约定案，FR37/D53；无代码变更）

- **决策 D53 / 新增 §9.20**：动态写与只读**端点分离**——`POST /v1/mutate`、scope `ref:mutate`、ACL operation `mutate`；`/v1/invoke` 与 `/v1/mutate` 互不接受对方 `sideEffect`，只读授权不自动升级为写授权。
- **`mcp-mutating` profile**：必填 `tool.inputSchemaDigest`、`idempotency {key, semantics}`、`result.materialization ∈ {required, optional}`、`auth.audience`、非空 `failure`；`reference` 严格等于 `<server.id>/<tool.name>`，`operation == tool.name`（比只读前缀匹配更严）。
- **幂等台账**：`[mutate].journal` append-only JSONL（单写者，`O_APPEND` + 每记录 fsync）；`intent` → 上游 → `result` 两阶段；键作用域 `(actor, domain, type, reference, operation, idempotencyKey)`，`requestDigest` 为紧凑 JSON 的 sha256。同键同摘要成功/失败重放、`in-doubt` 阻断、同键异摘要冲突、满额拒绝；崩溃窗口显式接受为 `in-doubt`，不自动重试。
- **两阶段审计**：每次写 attempt 两条（`intent`/`result`）；`intent` 失败不触达上游，`result` 失败返回 `500 AuditUnavailable` 但台账保留结论。不记录 `input`、上游正文与令牌。
- **witness/compensation 边界**：写成功后仍经 `/v1/resolve` + 参与者 `seed`/`produce` 产生 witness；`failure.compensation` 只是保留键，不做自动 saga/补偿，撤销另立写卡片。
- **配置与启动**：`[mutate].journal`（mutating profile 存在时必需）、`[mutate].max_entries`（缺省 10000，1..=1000000）；复用 `[materialize]`/`[[mcp_servers]]`；台账不可读/非法行、必填字段缺失、满额越界均 fail-closed。
- **规范层**：`contract-card.schema.json` 本轮不收紧（`mutating` 必填校验放在实现轮的 profile 校验），避免在无实现时先制造 schema 与运行期判定不一致。
- **文档落点**：`docs/requirements.md` FR37、`docs/design.md` §0 D53 与 §9.20、§10 测试表；`docs/thingref-governance.md` §13.1 阶段 5 与 §13.8；`docs/architecture.md` 附录 A 与 §2.9；`spec/ref/v1/reference-plane.openapi.yaml` 新增 `/v1/mutate`、`MutateRequest`、`Mutation`、409 响应与四个错误码。
- **合同校验**：OpenAPI 6 条 path、9 个 schema、14 个 response，本地 `$ref` 全部可解析（YAML 解析通过）。
- **跨仓库**：本轮无代码变更；实现将落在 `pact-ref`（台账模块、handler、`ref-auth` scope、`ref-core` 类型）与本仓库 `pact-ref-spec` 的 mutating 卡片校验，需在台账单独标注。
- **未决**：台账文件格式细节（记录字段命名/版本）、是否复用 `ref-audit` 的既有 sink、`max_entries` 满额后的运维归档方式，留待实现轮定案。
- **其余**：D1–D52 与 FR1–FR36 无实质变更；母本未改动。

## 2026-10-31（第 93 轮，`refd` 动态只读调用实现，FR36/D52）

- **跨仓库**：代码全部在 `pact-ref`（`crates/ref-core`、`ref-auth`、`ref-invoker`、`ref-client`、`adapters/mcp`、`services/refd` 与 vendored 快照）；本仓库改动为 `spec/ref/v1/reference-plane.openapi.yaml`、`crates/pact-ref-spec/src/contract.rs` 的只读卡片校验，以及需求/设计/架构/治理与台账。
- **决策 D52 / 新增 §9.19**：`POST /v1/invoke` 把动态读取收进治理平面——只读闸门（`capability.sideEffect = read-only`）、引用归属（`reference` 必须以 `capability.server.id + "/"` 开头且工具路径非空）、结果物化为内容寻址静态引用（`[materialize].dir/<sha256>.json`，`.tmp` + rename 原子写，返回 `resultRef` + `resultDigest`）。`resolve` 保持纯静态解析，动静分离。
- **调用链（已实现）**：解析 JSON（`deny_unknown_fields`）→ `domain`/`type`/`operation` 选择器与 `idempotencyKey` 校验 → 引用绝对 URI → 对象 ACL `invoke` → profile 存在（否则 `404`）→ scheme 白名单（否则 `400`）→ capability 存在 → `read-only`（否则 `403`）→ 引用归属（否则 `400`）→ `InvokerRegistry` 路由到 `mcp` adapter（JSON-RPC 2.0 `tools/call`，`spawn_blocking`，总上限 30s）→ 物化 → 审计。
- **规范层补校验**：`pact-ref-spec` 的 `validate_capability` 对 `side_effect = read-only` 的卡片要求 `idempotency`（键与语义非空）、`result.materialization = required`、`auth.audience` 非空、至少一个 tool schema 摘要；不满足即 `InvalidContract`。`pact-ref` 的 vendored 快照已同步，`sync-spec.sh --check` 输出 `vendored spec is in sync`。
- **配置与启动 fail-closed**：`[materialize]`（`dir` 必须存在且为目录，`max_bytes` 缺省 1 MiB、上限 16 MiB）；`[[mcp_servers]]`（`id` 为 `mcp://` URI、`endpoint` 默认仅 `https://`、`timeout_ms ≤ 30000`、可选 `bearer_token_env`、`allow_plaintext` 仅回环主机可设）。存在只读能力 profile 时，其 `server.id` 必须已登记且必须配置 `[materialize]`；`bearer_token_env` 未设置或为空即拒绝启动。
- **错误与审计**：上游明确失败 → `502 UpstreamFailure`（固定消息、`retryable=false`），审计 `status = failed`；`refd` 超时 → `504`；ACL/只读闸门 → `403`；引用格式/归属 → `400 InvalidReference`；未知字段/幂等键 → `400 InvalidRequest`。审计不记录 `input`、上游正文与令牌。
- **边界**：v1 不重试、不轮询、不缓存上游结果；`capability.timeoutMs` 不参与执行；`mcp-mutating` 与 `revoke` 仍在动态写阶段。
- **依赖与安全审视**：未新增第三方包（`adapters/mcp` 复用 workspace 既有 `ureq` 2、`sha2`、`serde_json`；`Cargo.lock` 仅增加 workspace 内部依赖边）。生产代码无 `unsafe`；`refd` 只在显式配置的 `[materialize].dir` 下写单个 `<sha256>.json`，不遍历、不删除；上游端点仅 `https://` 或显式回环明文；不跟随上游重定向（`ureq` 调用不启用 `redirects`）；结果读取有 `max_bytes` 上限；`bearer` 令牌只从环境变量取、不落日志/审计；`input` 不写审计；`refd` 仍不写 PACT 存储。`cargo audit`：0 vulnerability / 1 unmaintained（`unsafe-libyaml`，既有）。
- **验证**：`pact-ref` 全量 **120 passed / 0 failed**（`reference_plane` 24，其中 invoke 7；`pact-ref-spec` 45，含 5 项只读卡片校验；`ref-auth` 18）；`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --offline -- -D warnings`、`cargo audit` 通过。`pact` 侧 `cargo test --workspace --locked` **378 passed / 0 failed**（基线 373 + 只读卡片校验 5），`fmt`/`clippy` 通过。
- **合同校验**：OpenAPI 5 条 path、7 个 schema、13 个 response，57 个 `$ref` 全部可解析。
- **未决**：动态写阶段（`mcp-mutating`、审计边车、witness/compensation）、`git`/`oci` adapter、conformance runner 仍按台账推进。

## 2026-10-31（第 92 轮，`refd` OIDC 验证器实现）

- **跨仓库**：代码全部在 `pact-ref`（`crates/ref-auth/src/oidc.rs`、`services/refd/src/{config,state,jwks,handlers}.rs`、`services/refd/tests/reference_plane.rs`）；本仓库只改需求/设计/架构/治理与台账。
- **分层（D51）**：`ref-auth` 新增 `JwksSource`（`fetch() -> Result<Vec<u8>, String>`）与 `OidcAuthenticator`/`OidcSettings`，只做 JWT 验签、声明校验、`sub`→actor 与 `scope` claim 解析；不引入网络 IO，测试可注入固定 JWKS。`refd` 新增 `jwks.rs` 提供 `FileJwksSource`（常规文件、≤ 1 MiB）与 `HttpJwksSource`（`ureq` + rustls、超时、`redirects(0)`、2xx、响应 ≤ 1 MiB）。
- **验签与声明**：算法白名单 `RS256`/`ES256`/`EdDSA`，白名单外（含 `none`/`HS*`）拒绝；`crit` 非空拒绝；`kid` 匹配 JWK，无 `kid` 仅唯一可用密钥接受；`jsonwebtoken` 校验 `iss`/`aud`/`exp`/`nbf`（`leeway ≤ 60`）；`sub` 非空即 actor。
- **缓存与刷新**：`JwkSet` 按 `cache_ttl_seconds` 复用，密钥上限 64；未知 `kid` 触发一次强制刷新，`FORCE_REFRESH_MIN = 30s` 内限频并复用当前缓存。拉取/解析/空密钥集一律 `AuthError::Invalid`。
- **配置与启动**：`config.rs` 新增 `OidcAuthConfig` 与 fail-closed 校验（`issuer` https 且无 userinfo/fragment、`audience` 非空、`jwks_uri`/`jwks_file` 恰一、算法子集、`leeway ≤ 60`、`cache_ttl > 0`、`timeout > 0`），非法配置拒绝启动；`state.rs` 按 mode 装配。
- **请求路径**：`handlers.rs` 把 `authenticate` 移入 `tokio::task::spawn_blocking`，失败统一走 `unauthorized()`；所有 `401` 带 `WWW-Authenticate: Bearer realm="pact-ref"`。合法令牌缺端点 scope 仍为 `403`。
- **未变**：`/readyz` 仍只反映审计 sink 健康；OIDC 不做 discovery/撤销/内省，`refd` 不访问 PACT 存储；`static`/`deny` 模式行为不变。
- **依赖与安全审视**：新增 `jsonwebtoken` 9.3.1、`base64` 0.22.1（`ref-auth`），`ring` 0.17.14（仅测试生成 Ed25519 密钥），`refd` 复用第 91 轮已引入的 `ureq` 2（rustls）。生产代码无 `unsafe`；JWKS 只从配置指定的文件或 HTTPS 地址读取，HTTPS 不跟随重定向、有超时与 1 MiB 上限；令牌与凭据不写日志、不进审计；认证失败统一 `401` 不区分原因。`Cargo.lock` 因新依赖新增 22 个包。`cargo audit`：0 vulnerability / 1 unmaintained（`unsafe-libyaml`，既有）。
- **验证**：`pact-ref` `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`、`cargo test --workspace --offline` 通过，合计 **99 passed / 0 failed**（`ref-auth` 18，其中 OIDC 11；`refd` 集成 17，其中 OIDC 3）；`cargo audit` 0 vulnerability / 1 unmaintained。
- **未决**：动态只读/写阶段、`git`/`oci` adapter、conformance runner 仍按台账推进。

## 2026-10-31（第 91 轮，`refd` resolve/describe 实现）

- **跨仓库**：代码全部在 `pact-ref`（`ref-core` / `ref-auth` / `ref-audit` / adapters / `services/refd`）；本仓库只改 OpenAPI 与状态文档，无 `pact` 服务代码变更。
- **`ref-core` 契约对齐**：`ResolveRequest.actor` 由服务端注入（请求体不再接受）；`Resolution.handle` 改为 `Option`；`ContractDescriptor` 扩为白名单投影；`AuditEvent` 补 `domain` / `typeName` / `pactTokenId`；`AuditSink::record` 返回 `Result` 并新增 `available()` 就绪检查。
- **`ref-auth`**：`DenyAuthenticator`（默认）、`StaticTokenAuthenticator`（显式 `allow = true`、常量时间比较、scope 集合）、`Acl` 求值器（维度 `(principal, domain, type, scheme, operation)`，并集允许、默认拒绝）。端点 scope 与对象 ACL 是两层，任一拒绝即 `403`。
- **`ref-audit`**：`FileAuditSink` 以 append-only JSONL 写入并逐条 flush；`available()` 以文件锁状态反映 sink 健康，供 `/readyz` 使用。
- **`refd`**：`axum` 0.7 路由 `/healthz`、`/readyz`、`/v1/resolve`、`/v1/describe`；处理顺序为 trace id → 方法 → 认证 → 审计上下文 → scope → `Content-Type` → 64 KiB 请求体 → JSON → selector/reference → 对象 ACL → profile → scheme 白名单 → adapter（`spawn_blocking` + 30s 上限）→ 摘要比对 → 响应 → 审计。审计失败默认覆盖为 `500 AuditUnavailable`，`[audit] fail_open = true` 才放行。`[auth] mode = oidc` 启动即失败。
- **adapter**：`file` 只接受 `file://` / `file://localhost` 绝对路径，做百分号解码、拒绝 `..` 并 `canonicalize` 到配置根目录内；`https` 用 `HEAD` 不取正文，主机 allowlist 默认拒绝，拒绝 userinfo 与重定向，摘要取 `Digest: sha-256`（base64 或 64 位十六进制）或 `X-Checksum-Sha256`。
- **契约放宽（本仓库）**：`ContractDescriptor.required` 去掉 `lifecycle`，改为 `[contractId, version, mediaType, reference]`。`ContractCard` 的 `spec.lifecycle` 是可选项，原 required 会拒掉合法卡片的投影；其余 required 不变，投影白名单规则不放松。
- **依赖与安全审视**：新增 `axum` 0.7、`tokio` 1（`rt-multi-thread` / `macros` / `net` / `signal` / `time`）、`http-body-util` 0.1、`toml` 0.8、`ureq` 2（rustls），`tower` 仅测试用。生产代码无 `unsafe`；`refd` 不访问 PACT 存储；`file` adapter 只读 canonicalize 后的根内文件；`https` adapter 不跟随重定向、不取正文；审计不记录 `handle`、凭据、请求体或内部 endpoint。`cargo audit`：0 vulnerability / 1 unmaintained（`unsafe-libyaml`，既有）。
- **验证**：`pact-ref` `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace --locked` 通过，合计 **87 passed / 0 failed**（其中 `refd` 端点集成测试 14 项）；真实二进制冒烟：`/healthz`、`/readyz` 均 200，`/v1/resolve` 返回 digest 与 handle，`/v1/describe` 不含 owner，无凭据 401，审计落两行。
- **CI 修正**：`refd` 由骨架变为必须读取配置后，`pact-ref` 的 `test` job 中原 `cargo run -p pact-refd` 会以退出码 1 结束；改为写入临时 `refd.toml`（回环 + 明文 + 审计路径）、后台启动二进制并探测 `/healthz`、`/readyz`，失败即 job 失败。
- **未决**：OIDC（JWT/JWKS）延后，`mode = oidc` 启动失败；`git`/`oci` adapter 仍为占位；限流、mTLS、配置热加载不在 v1。

## 2026-10-31（第 90 轮，`refd` resolve/describe 端点契约与鉴权定案）

- **解除台账阻塞**：P3 首项原为“`refd` resolve/describe 端点 / 阻塞于端点契约与鉴权设计”。本轮定案端点契约与鉴权，规范产物为 `spec/ref/v1/reference-plane.openapi.yaml`，设计定案写入 `docs/thingref-governance.md` §13.6。
- **六条不变量**：`refd` 永不调用 `pactd`；主体来自验证后的凭据 `sub`，请求体不接受 `actor`；`domain`/`type` 对 `refd` 不透明；v1 只解析 URI（`∅`/`null`/Identifier/裸路径 → `400 InvalidReference`）；响应不含制品正文、内部 endpoint、凭据；默认拒绝。
- **传输与版本**：业务端点 `/v1/resolve`、`/v1/describe`；`/healthz`、`/readyz` 不带版本；wire 版本仍为 `pact.ref/v1`，破坏性变更升 `/v2` 并同步 `API_VERSION`。仅 `application/json`（`415`），请求体 ≤ 64 KiB（`413`），单请求 30s（`504`），`Cache-Control: no-store`，`X-Trace-Id` 头（`[A-Za-z0-9._:-]{1,128}`）。
- **身份与凭据**：`oidc`（JWKS、校验 `iss`/`aud`/`exp`/`nbf`、算法白名单 RS256/ES256/EdDSA）、`static`（显式 `allow = true`，常量时间比较）、默认 `deny`；生产要求 TLS，401 带 `WWW-Authenticate: Bearer realm="pact-ref"`。
- **授权两层**：端点 scope `ref:resolve` / `ref:describe`，加上对象 ACL（维度 `(principal, domain, type, scheme, operation)`，operation ∈ `resolve:read` / `resolve:verify` / `describe`），并集允许、默认拒绝。PACT token read gate 与制品 ACL 独立，参与者必须同时满足。
- **describe 白名单投影**：只透出 `contractId`/`version`/`mediaType`/`reference`/`lifecycle` 及可选 `status`/`artifact`/`capability`/`failure`/`verification`/`consumerScope`；不透出 `metadata.owner|labels|description`、`parties`、`capability.server`、`capability.auth.credentialSource`、`resolution.endpoint`，也不自动透出未来新增字段。
- **错误模型**：统一信封 `{code, message, traceId, retryable, detail?}`；`401`（凭据无效）与 `403`（scope/ACL 拒绝）严格区分，`403` 不透露对象存在性；`intent=verify` 只校验存在性与摘要，禁止返回 `handle`。
- **审计 fail-closed**：每条已认证请求一条 `AuditEvent`，不记录 handle 明文、`Authorization`、请求体、内部 endpoint；审计写失败 → `500 AuditUnavailable`（`[audit] fail_open = true` 需显式配置）。
- **OpenAPI 重写**：删除旧的 `/resolve`、`/invoke`、`/revoke` 与请求体 `actor`；新增 `/healthz`、`/readyz` 与 v1 组件（`ResolveRequest`/`Resolution`/`DescribeRequest`/`ContractDescriptor`/`Error` 及 12 个错误响应组件）；invoke/revoke 延后到动态阶段另行定案。
- **实现映射与未决项**：`ref-core` 移除请求体 `actor`、`Resolution.handle` 改 `Option`、`ContractDescriptor` 扩为投影；`ref-auth` 新增 OIDC/静态/ACL；`refd` 建议 `axum` + `tokio`（需单独依赖与安全审视）。限流、mTLS、配置热加载、Identifier 注册表不在 v1。
- **无代码变更**：本轮仅文档与规范；跨仓库仅有 `pact-ref/docs/architecture.md` 的 API 段落与“引导阶段下一步”同步（不涉及 `pact-ref` 代码与 vendored 快照）。
- **验证**：`yaml.safe_load` 解析通过（paths 4 项、schemas 5 项），遍历全部 `$ref` 均解析成功；无代码回归，沿用第 88 轮 373 passed 基线。
- **安全审视**：无新增依赖、无代码、无网络/进程/文件写变化；设计层面明确默认拒绝、凭据不入请求体、审计 fail-closed；`cargo audit` 0 vulnerability / 2 unmaintained（沿用）。

## 2026-10-31（第 89 轮，物引治理 skill；规范文档纠偏）

- **新增 `skills/pact-thingref-governance/SKILL.md`**（`name: pact-thingref-governance`，v0.1.0）：面向使用者的物引治理技能，覆盖绑定、契约卡、tombstone、可见性、`declarationDigest` 与显式重载六个主题；含适用闸门（不启用项与去向）、六条不变量、每主题的最小骨架与验收命令、工具探测、采集槽位、lint/digest 闭环、完成标准、输出模板、失败模式与不承诺边界。
- **技能不引入新编号**：工作流阶段用名称而非 `S*` 编号；`S0`–`S7` 仍专属 `pact-domain-authoring`（`docs/architecture.md` 附录 A），避免同一字母序列两处含义。
- **验收基于真实二进制**：用 `target/debug/pact` 复跑 `ref digest`（输出与 golden 一致）、`ref lint`（0 error / 0 warning，exit 0）、`--strict`（exit 0），以及篡改摘要后的 `ref.bindings.digest_mismatch`（exit 1）。
- **规范纠偏**：`spec/ref/v1/declaration-digest.md` §4 原写“运行期比对尚未启用”，与 D49 第二步实现及 `docs/manual.md` §7.14 冲突；改为已实现并注明 `[ref_digest].mode` 三档行为与比较边界。
- **无代码变更**：不触碰 `crates/`、`spec` schema 与 daemon 配置；跨仓库无改动（`pact-ref` vendored 快照不受影响）。
- **验证**：文档与技能文件，仅需现有回归作为基线；定向命令见上条。

## 2026-10-31（第 88 轮，D50：WASM 本地入口校验与绑定注入）

- **D50 定案**：把入口票面语义从 `pact-http::VisibilityProfiles::validate_thing_ref` 抽到 `pact-ref-spec::entry::validate_entry` 纯函数，REST 与 WASM 共用；`EntryRef` 保留三态（`Missing` / `Absent` / `Present`），`EntryError` 由规范层统一。
- **REST 委托**：`pact-http` 的 `validate_thing_ref` 只做三态适配并查询本域绑定后委托；`RefEntryError` 改为 `pub use pact_ref_spec::EntryError`；错误码映射与 e2e 行为不变。删除本地 `scheme_is_unknown`，统一由规范层实现。
- **WASM 注入**：`PactHost` 增加内存 `ref_bindings: BTreeMap<String, RefBindings>` 与 `set_ref_bindings` / `remove_ref_bindings` / `clear_ref_bindings`；`set_ref_bindings` 解析 JSON 后立即 `validate`，非法输入映射为 `BadJson` / `InvalidBindings`。WASM 不读文件、不引入 YAML 解析器。`prepare_seed` / `prepare_produce` 在取会话前调用同一纯函数，成功写规范化票面；`prepare_produce` 的前驱 JSON 解析仍先于票面校验。
- **WASM 三态语义**：JS/WASM 无法表达“字段缺失”，`thing_ref` 省略与 `null` 均映射为 `Absent`（`∅`），因此 WASM 路径不产生 `ThingRefMissing`；错误映射为 `ThingRefRequired` / `ThingRefUnpinned` / `InvalidThingRef`。
- **JS 宿主**：`openPactHost({ refBindings })` 接受对象、对象数组或 JSON 字符串（也支持 `PACT_WASM_REF_BINDINGS`），在实例加载/恢复前注入；`PactHost` 暴露 `setRefBindings` / `removeRefBindings` / `clearRefBindings`。
- **边界**：绑定只影响本进程内存，宿主重启后需重新注入；ContractCard `requiredForArchive` 与 Archive 检查仍只在 REST 边界（D47），WASM 不加载卡片、不做归档钉版；多用户 ACL/读审计仍不在 WASM。
- **文档**：`docs/design.md` §0 新增 D50、§9.12 未覆盖路径、§9.17 实现记录、§10 测试表与 §16.4 维护说明；`docs/architecture.md` 正文与附录 A 第 88 轮；`docs/requirements.md` FR21/FR30/FR31；`docs/manual.md` §2.7 与 §7.11；`docs/thingref-governance.md` §8.1/§8.2/§13.1；README 中英文本地嵌入描述改为用途视角。
- **跨仓库**：`pact-ref-spec` 变更同步到 `pact-ref/vendor/pact-ref-spec`（`scripts/sync-spec.sh --source …`，`--check` 通过）；`pact-ref` `fmt`/`clippy`/`test` 全绿。
- **验证**：`pact-ref-spec` 40 passed、`pact-wasm` 14 passed、Node 测试 8 passed、wasm32 clippy 与 `ci-smoke` 通过；REST 侧 `pact-http` 单测与 e2e 全绿。
- **安全审视**：新增依赖 `pact-wasm → pact-ref-spec` 为纯 crate（无 IO/网络/`unsafe`）；WASM 仅接收宿主注入的 JSON，不新增文件 IO、进程或网络能力；`cargo audit` 结论沿用。

## 2026-10-31（第 87 轮，D49 第二步：`[ref_digest]` 运行期比对实现）

- **配置**：`pact-http::config` 新增可选 `[ref_digest]`（`deny_unknown_fields`）与 `RefDigestMode`（serde `rename_all = "lowercase"`，values `off` / `warn` / `enforce`，缺省 `warn`）；`ref_digest_mode()` 供 daemon 启动注入 `ServerState`。未知键或未知模式在加载配置时拒绝。
- **比较点**：`load_domain` 在 `authorize_key` 之后、`engine.load_domain` 之前调用 `check_declaration_digest`；摘要缺失（该域无绑定或绑定未记录）→ 任何模式都不比较；声明解析失败 → 返回既有 `Syntax` / `WellFormedness`，不改写错误语义。
- **三模式**：`off` 直接放行；`warn` 打一条 stderr 警告（`expected` / `actual` / `digestVersion`）并继续加载；`enforce` 返回 `400 DeclarationDigestMismatch`，`detail = {domain, expected, actual, digestVersion}`。拒绝发生在引擎调用前，域未装载、无事件写入。
- **启动恢复 / D48 重载不比较**：摘要约束的是“声明注册时对应哪版绑定”，既有实例持声明快照；重载只替换绑定规则，不追溯已登记声明。
- **可测试性**：比较逻辑抽为纯函数 `digest_mismatch(mode, expected, name, declaration)`，单测覆盖三模式、缺失摘要、外来域与解析失败；新增 `ref_digest_e2e`（真实 daemon）5 项。
- **编号登记**：`[ref_digest]`、`[ref_digest].mode`、`DeclarationDigestMismatch` 于 `docs/architecture.md` 附录 A 第 87 轮登记；`docs/manual.md` §2.2 / §7.14 / §8.2 / §9.4 / §10 与 `dist/pact.container.toml` 同步。
- **跨仓库**：无 `pact-ref` 代码改动；`spec/ref/v1/declaration-digest.md` 不参与 `scripts/sync-spec.sh` 的 crate 快照。
- **验证**：全量 `cargo test --workspace --locked` **363 passed / 0 failed**；`cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。

## 2026-10-31（第 86 轮，`declarationDigest` v1 第一步实现）

- **D49 定案并实现第一步**：摘要对象取语义接口（域名 + `types` + `preconditions` + `gates` 含条件 + `seed_authority`），规范化与 golden 向量冻结在 `spec/ref/v1/declaration-digest.md`；实现为 `pact-analysis::digest` 纯函数（新增 workspace 既有 `serde_json`/`sha2`），CLI 与未来 `pact-http` 共用。
- **`gates` 排序修正**：`BTreeMap<(Participant, Action, TypeName), …>` 的枚举序是 `Read`/`Consume`/`Produce`，与协议要求的字符串字典序不同；序列化前显式按 `(participant, action.as_str(), type)` 重排，并在规范中冻结。
- **CLI**：`pact ref digest` 输出 `sha256:<64hex>`；JSON 为 `{declarationDigest, algorithm:"sha256", digestVersion:1}`；域名取 `--name` 或文件 stem，非法（不匹配 `[A-Za-z0-9_-]{1,128}`）返回 `Config` 错误。
- **lint**：`pact ref lint --declaration` 具备域名时计算并比对；新增 error `ref.bindings.digest_mismatch`，缺失仍为 warning。既有测试夹具的摘要同步为真实值。
- **未实现（登记为 D49 第二步）**：`[ref_digest].mode = off|warn|enforce`（默认 `warn`）、`load_domain` 比对、`enforce` 的 `400` 错误码；配置项与错误码须先登记。
- **设计文档**：D41 摘要边界、§9.12 未覆盖路径、§9.16 新节、§10 测试表同步；治理 §8.1/§13.1/§13.4.2/§13.4.2.1 标注“已确认、第一步已实现”。
- **跨仓库**：无 `pact-ref` 代码改动；`scripts/sync-spec.sh` 只同步 `pact-ref-spec` crate，`spec/ref/v1` 文档不参与快照。
- **验证**：全量 `cargo test --workspace --locked` **356 passed / 0 failed**（新增 7 项：digest 5、lint 单测 1、e2e 1）；`cargo fmt --all -- --check` 与 `cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。
- **安全审视**：无新增包（`pact-analysis` 仅启用 workspace 既有 `serde_json`/`sha2`）；`digest.rs` 为纯函数，无 `unsafe`/网络/进程/文件 IO，CLI 仅只读声明文件；`cargo audit` 0 vulnerability / 2 unmaintained（沿用）。

## 2026-10-31（第 85 轮，`declarationDigest` 候选定义，待确认；无代码变更）

- **触发**：用户要求先确认摘要对象与启用方式，再进入实现。
- **候选定义**：写入 `docs/thingref-governance.md` §13.4.2.1。摘要对象取“语义接口”（域名 + `types` + `preconditions` + `gates`（含条件表达式）+ `seed_authority`），规范化 UTF-8 紧凑 JSON，条件用自定文法文本而非 `serde` 派生表示；摘要 = `sha256:` + 小写 hex。注释、空白、字段顺序、文件名、实例名、绑定/卡片内容不参与。
- **候选启用方式**：第一步 `pact ref digest` + `pact ref lint` 比对（摘要存在且不一致 = error）；第二步运行期 `[ref_digest].mode`（默认 `warn`、`enforce` 显式开启），仅在 `load_domain` 比较；启动恢复与 D48 重载不比较。
- **格式**：字段保持 `sha256:<64hex>`，v1 语义冻结在待新增的 `spec/ref/v1/declaration-digest.md`；不改 schema、不加版本字段，未来换义须新规范版本与新字段/编号。
- **未做**：未登记新 FR/D、配置项或错误码，未改代码；确认后再登记。预计落点 `pact-analysis`（纯函数）、`pact-cli`（digest 子命令与 lint 比对）、`pact-http`（第二步运行期）、`spec/ref/v1`（规范与 golden），并需在台账标注 `pact-ref` 规范文档同步。
- **文档**：治理 §13.4.2.1 与 §13.4.2 的确认指引；`todo.md` 仍为 `[!]`，证据补“候选已给出，待用户确认”。

## 2026-10-30（第 84 轮，B1 显式重载实现）

- **实现（FR33/D48）**：新增 `pact-http::reload` 模块与 `POST /ref/reload`；daemon 启动注入 `RefReloadConfig`（绑定/卡片路径 + `<data_dir>/ref-reload.jsonl`）。事务顺序固定为认证 → 当前域 ACL → 加载校验 → 新域 ACL → 影响预检 → 审计 → 原子替换；`dryRun` 在审计后停止。
- **原子快照**：`ServerState.visibility` 改为 `RwLock<Arc<VisibilityProfiles>>`，请求入口取一次快照并全程复用；替换只换 `Arc`，在途请求持旧规则。`counter`/`archive`/`consume`/入口校验均改用该快照。
- **审计摘要同源**：`VisibilityProfiles` 记录本次加载实际读取文件的 `path`/`sha256`/`kind`，审计直接写该清单，避免二次读取造成 TOCTOU；审计写失败拒绝替换（含拒绝路径）。
- **影响预检**：`newly_blocked_tokens` 只统计“旧规则通过、新规则因 `PinningRequired { at_archive: true }` 阻断”的存量 Counter token，按 `(domain, instance, type)` 聚合计数，不输出票面值。
- **CLI/client**：`Client::reload`、`pact ref reload [--dry-run]`；JSON 输出 `RefReloadResp`，人类输出含逐组阻断计数。
- **测试**：`pact-http` 单测 3 项（审计 JSONL/fsync、审计目标为目录、锁占用 `409`）；`ref_reload_e2e` 7 项；CLI e2e 1 项；全量 **349 passed / 0 failed**；`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过。
- **文档**：`requirements.md` FR33/FR32 状态与边界、`manual.md` §2.2/§7.11/§7.12/§8.1/§8.2/§9.4、`architecture.md` 正文、治理 §8.1/§13.1/§13.4.1/§13.5、`todo.md`。
- **未变更**：`pact-ref-spec` 与 `pact-ref` 未改动，无需 vendored 快照同步；不做文件监听/SIGHUP/自动回滚/按域灰度；不提供 MCP 工具。
- **安全审视**：无新增依赖；新增文件读写限于配置源与 `<data_dir>/ref-reload.jsonl`（append+fsync），无 `unsafe`、网络或进程副作用；控制面端点要求认证与 ACL 通配授权，审计失败 fail-closed；`cargo audit` 结论沿用。

## 2026-10-30（第 83 轮，B0 契约卡加载实现）

- **实现（FR32/D47）**：`pact-http` 新增 `[ref_contracts].paths` 配置与 `VisibilityProfiles::load_with_contracts`；启动时解析并 `ContractCard::validate`，按 `contract_id()` 索引；重复 `contract_id()`（无论内容）即启动失败。加载绑定后扫描原始规则中的 `contract` 引用，未配置 `[ref_contracts]` 或引用缺失均拒绝启动；无 `contract` 引用的部署仍免配置。
- **归档判定**：`validate_archive_thing_ref` 在绑定 `pinning` 基础上并入卡片 `reference.pinning.requiredForArchive` / `artifact.digestPolicy: required-at-archive`，只加严、不放宽；卡片索引缺失时 `InvalidThingRef` fail-closed。写入路径不受卡片钉版影响；`∅` 与已钉版快速放行。
- **配置校验**：`[ref_contracts]` 已配置但 `paths` 为空 → `ConfigError::Invalid`；daemon 启动日志输出 RefBindings/ContractCard 计数。
- **测试**：新增 `pact-http` 单测 9 项（卡片加载与引用闭环、无引用免配置、缺失/重复拒绝、卡片钉版阻断归档、卡片不放宽绑定、索引缺失 fail-closed、配置空 paths）+ `visibility_e2e` 2 项（卡片钉版阻断未钉版 consume、重复版本拒绝启动）；全量 **338 passed / 0 failed**；`cargo fmt --all -- --check` 与 `cargo clippy -p pact-http --all-targets --locked -- -D warnings` 通过。
- **安全审视**：无新增依赖；仅新增配置路径读取与 YAML 解析，无 `unsafe`、网络、进程或写入副作用；`cargo audit` 结论沿用。
- **跨仓库**：未改动 `pact-ref-spec`，故无需同步 vendored 快照；`pact-ref` 侧无改动。
- **未实现**：B1 显式重载（FR33/D48）仍待实现；`declarationDigest` 仍待决策；`artifact.kind`/`mediaType`、`lifecycle` 运行期检查不在 B0。
- **文档同步**：`requirements.md` FR32 状态、`design.md` §9.14 与 D47、`manual.md` §2.2/§7.11、`architecture.md` 正文、治理 §8.1/§13.1/§13.4.1/§13.5、`todo.md`。

## 2026-10-29（第 82 轮，ContractCard 加载与重载编号登记，无实现变更）

- **触发**：用户确认 §13.5 方案，要求先登记 B1 的 FR/D 编号。
- **新增编号**（已先登记于 `docs/architecture.md` 附录 A）：
  - `FR32` / `D47`：ContractCard 运行期加载与版本不可变（B0）——可选 `[ref_contracts].paths`、`contract_id()` 索引、按数据触发闭环、同版本不可变、consume 钉版并集（只加严）。
  - `FR33` / `D48`：ContractCard 与 RefBindings 显式重载（B1）——`POST /ref/reload` + `pact ref reload`、卡片与绑定同事务、原子换索引、`dryRun` 影响报告、`ref-reload.jsonl` 审计、ACL 覆盖每个绑定域；不做文件监听/SIGHUP/自动回滚。
- **需求侧**：`docs/requirements.md` 新增 FR32/FR33；FR30/FR31 的“未生效项”指向新编号；`requirements-change.md` 第 40 轮。
- **设计侧**：`docs/design.md` §0 新增 D47/D48；未改架构正文（能力尚未实现，只在附录 A 登记编号）。
- **治理侧**：`docs/thingref-governance.md` §13.5 的 B0/B1 标注已登记编号；B2 仍按需再评估。
- **变更范围**：仅文档；无代码、测试、schema 或配置变更。实现顺序仍为 B0（FR32/D47）→ B1（FR33/D48）；B1 依赖 B0 的索引与不可变性。
- **待用户确认**：`declarationDigest`（FR27 边界）仍待决策，不影响本轮登记。

## 2026-10-29（第 81 轮，ContractCard 生命周期方案，无实现变更）

- **触发**：用户追问 §13.4.1 第 4 条“首版只在启动时加载、卡片变更需重启”是首版约束还是终身形态；域声明已有迁移式生命周期（D27 实例快照声明、D39 显式交接），卡片能否取得类似效果。
- **结论**：卡片不承载状态、不进事件日志，不能用域交接类比；其等价物是**同版本不可变 + 版本化发布 + 原子激活**。卡片的 `metadata.id@version` 与绑定 `contract` 的 `id@version` 精确引用已内建版本化，“变更卡片”的正确定义是发新版本，而非改旧文件。
- **新增治理 §13.5**：选项 A（仅启动加载）、B（版本化 + 显式重载，推荐）、C（文件监听，不推荐）、D（按实例冻结绑定/卡版本，仅按需）。推荐分阶段：B0 随 §13.4.1 实现（重复 `id@version` 即启动失败）、B1 单独立项 `POST /ref/reload`（卡片与绑定同事务、原子换索引、审计 `by`/`ts`/每文件摘要、可选预检报告）、B2 视审计需求再评估按实例冻结。
- **回溯性加严**：按 D46，归档钉版在 consume 时按当前规则判定；切换绑定到更严卡版本会让存量未钉版 token 不可归档。缓解为 B1 的预检报告 + 发布说明；“只影响切换后产生的 token”属 B2/选项 D 一类，不在 B1 范围。
- **显式不做**：文件监听自动热加载、SIGHUP 隐式重载。
- **变更范围**：仅 `docs/thingref-governance.md` §13.1/§13.4.1 与新增 §13.5、`todo.md`；无代码、测试、schema、配置变更，未新增 FR/D 编号。
- **待用户确认**：首版仅为激活机制；是否采纳 B0 同版本不可变；是否单独立项 B1；是否接受不做文件监听。

## 2026-10-29（第 80 轮，待决方案澄清，无实现变更）

- **触发**：用户追问“决策一的选项 C 是否仍具 B 的闭环与可扩展收益”“若 `[ref_contract]` 可选且决策二选 C，摘要误算是否直接阻塞域加载”。
- **收紧 C 的语义**：“可选加载”改为**按数据触发闭环**——任一 RefBindings 规则声明了 `contract` 时，必须配置 `[ref_contracts]` 且能解析，否则启动失败；完全没有 `contract` 引用的部署无需配置。结论：在声明 contract 的部署上闭环等价于 B，可扩展能力（`artifact.kind`/`mediaType`、生命周期检查）与 B 相同；B 仅多出“无 contract 也挂卡”的形式统一，无治理收益。
- **明确摘要误算的失败模式**（治理 §13.4.3）：`[ref_contracts]` 与摘要比对相互独立；摘要比对只看 `metadata.declarationDigest` 与 `[ref_digest].mode`。首版默认 `warn`（只记 `expected`/`actual`/算法版本，`load_domain` 成功），`enforce` 为显式可选项；摘要缺失、算法版本不受支持、daemon 启动恢复既有域均不阻塞。仅“`enforce` + 摘要存在但不匹配”返回 `400`。
- **误算来源与缓解**：工具/运行时实现漂移、规范化定义歧义、手写摘要、真实声明变更；缓解为共用 `pact-ref-spec` 实现、冻结规范化规则、golden/conformance、lint 作为提交前门禁、错误报告 `expected`/`actual`。
- **变更范围**：仅 `docs/thingref-governance.md` §13.4.1 / §13.4.2 / 新增 §13.4.3 与 `todo.md` 表述；无代码、测试、schema 或配置变更，未新增 FR/D 编号。
- **待用户确认**：C 的“按数据触发闭环”是否采纳；摘要对象（语义接口 vs 规范化 AST）、两步走、运行期首版默认 `warn`、摘要字符串是否带算法版本前缀。

## 2026-10-29（第 79 轮，待决方案分析，无实现变更）

- **背景**：P1 剩余两项（ContractCard 运行期加载、`declarationDigest`）缺前置决策。用户要求给出方案与收益/代价。
- **记录**：分析写入 `docs/thingref-governance.md` §13.4（不新增需求/设计编号）。ContractCard 推荐“可选 `[ref_contracts]` + 启用即 fail-closed + 只加严”；`declarationDigest` 推荐“语义接口摘要 + 工具/lint 先行、运行期后置”。
- **变更**：仅文档。`todo.md` 两项状态与依据已指向该节；无代码、测试或 schema 变化。
- **待用户确认**：两项推荐是否采纳；确认后再登记新需求/设计编号并实现。

## 2026-10-29（第 78 轮，形态 / 未知 scheme / 钉版运行期强制）

- **触发**：`todo.md` P1。RefBindings 的 `value`、`onUnknownScheme`、`pinning` 此前只在 `pact ref lint` 生效，运行期不强制。
- **决策（D46）**：三类策略继续在 REST 边界执行；`pinning: required` 在 seed/produce 拦，`required-at-archive` 在 consume（进入 Archive 的唯一转换）拦；consume 只读 `merged_rule` 的 `pinning`，不重跑写入期规则，避免规则收紧后历史 token 永久无法归档。ContractCard 等价字段需先解决卡片加载来源，拆分独立事项。
- **规范层**（`pact-ref-spec`）：
  - `resolve` 对“未知 scheme”按策略分流：`reject` 仍直接拒绝，`allow` / `treat-as-identifier` 放行给调用方；
  - 新增 `merged_rule`（只合并继承链、不做校验）供归档检查；
  - 新增 `classify_identifier`（重分类）与 `pinned_digest`（识别 `sha256:` / `sha256=` 的 64 位小写摘要）；
  - `validate_rule` 增加 `required=true` 与 `value=none` 矛盾检查；
  - 新增错误 `ValueFormMismatch`、`PinningRequired`。
- **接口层**（`pact-http`）：`validate_thing_ref` 增加未知 scheme 分流、`value` 形态检查与写入钉版；新增 `validate_archive_thing_ref`；`consume_inner` 在授权后、引擎前调用 `enforce_archive_pinning`（从 `engine.counter` 取待消费 token）。错误码新增 `ThingRefUnpinned`（`400`）。
- **测试**：`pact-ref-spec` 新增 4 个单测（钉版识别、未知 scheme 三分支、`merged_rule`、矛盾配置）；`pact-http` 新增 4 个单测（未知 scheme、`value`、写入钉版、归档钉版）；`visibility_e2e` 新增 2 个端到端（写入与 consume 的 `ThingRefUnpinned`、钉版引用成功归档），该文件 **9 passed**；`pact-ref-spec`+`pact-http` 定向 **52 + 33 passed**。
- **未做**：ContractCard `digestPolicy` 运行期加载（需先定来源/配置）；`declarationDigest` 漂移检测（需用户确认摘要算法）；WASM 入口校验；conformance runner。均登记 `todo.md`。
- **文档**：FR31、D46 与 §9.13；同步 FR30 未生效项、§9.12、§10 测试策略、manual §7.11、治理 §8.1/§13.1、架构附录 A 与正文。

## 2026-10-28（第 77 轮，ThingRef 入口校验）

- **触发**：第 76 轮审视的最高优先级缺口——`POST /seeds` / `POST /tokens` 直接透传 `thing_ref`，URI profile 与 RefBindings 策略只在视图过滤和离线 lint 中生效。用户要求按建议顺序开始修复。
- **边界定案（D45）**：校验放在 REST 写入边界（`pact-http`），在引擎调用前完成；`pact-engine` / `pact-store` / `pact-core` 仍不依赖 `pact-ref-spec`，票面保持不透明。CLI 与 MCP 经同一 client 自动继承；WASM 本地宿主另立事项。
- **规范层修复**（`pact-ref-spec`）：
  - `has_userinfo` 只拒绝含 `:` 的 userinfo（密码型）；`git+ssh://git@example.com/...` 恢复合法；
  - 新增 `is_valid_identifier`，强制执行 `1*(ALPHA/DIGIT/_/-/./:/@/#)`；新增 `SpecError::InvalidIdentifier`；
  - 新增硬上限 `HARD_MAX_REFERENCE_LENGTH = 65_536`，`RefBindings` 的 `maxLength` 不得超过它。
- **请求三态**（`pact-http::dto`）：`SeedReq` / `ProduceReq` 的 `thing_ref` 由 `Option<String>` 改为 `ThingRefField(Option<Option<String>>)`；自定义 `Serialize`/`Deserialize` 保留“字段缺失 vs 显式 `null`”。client 构造请求时显式发送该字段（缺省为 `null`）。
- **校验实现**（`pact-http::visibility`）：新增 `VisibilityProfiles::validate_thing_ref` 与 `RefEntryError`（`Missing` / `Invalid(SpecError)`）。顺序为三态 → `classify_with_limit(65536)` → `RefBindings::resolve`（`required`/`allowIdentifier`/`schemes`）→ 有效 `maxLength`（缺省 2048）→ 返回规范化值。
- **接线与错误码**（`pact-http::server`）：`seed_inner` / `produce_inner` 调 `validate_entry_thing_ref`；`Missing` → `ThingRefMissing`、`ReferenceRequired` → `ThingRefRequired`、其余 → `InvalidThingRef`，均 `400`。删除旧的 `fn thing_ref` 透传。
- **行为变化**：域加载 RefBindings 后默认只接受 URI（旧 Identifier 需显式 `allowIdentifier: true`）；未加载绑定的域仍允许 Identifier（回归锚点）。
- **测试**：`pact-ref-spec` 新增 userinfo 与 Identifier 文法单测；`pact-http` 新增三态序列化与绑定策略单测；`visibility_e2e` 新增 `entry_validation_structural_rules`、`entry_validation_applies_bindings_policy`（原始 HTTP 缺失字段、`null`、userinfo、裸路径、空格 Identifier、超长、大写 scheme 规范化、`required` 拒绝 `∅`、Identifier/scheme/maxLength 拒绝）。
- **回归修正**：全量测试暴露 `pact-runtime` 的 `runtime_e2e` 卡死——`mat` 域 RefBindings 只声明 `visibility: domain`，但场景沿用旧 Identifier（`task-1`、token ULID），新入口校验按“有绑定即默认 URI”拒绝种子，编排器持续重试。按既有语义给 fixture 补 `allowIdentifier: true`（不放松规则本身），`runtime_e2e` **5 passed**。该现象确认了行为变化的影响面：已有 visibility-only 绑定的域若仍写 Identifier，升级后需显式放行。
- **验证**：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过；`cargo test --workspace --locked` **317 passed / 0 failed**（50 个 test binary）。
- **安全审视**：无新增依赖（`Cargo.toml`/`Cargo.lock` 未改）；生产代码新增仅纯校验，无 `unsafe`/网络/进程/文件写入；`cargo audit` **0 vulnerability / 2 unmaintained**（`unsafe-libyaml`、`rustls-pemfile`，沿用）。错误文本回显的票面值已通过控制字符/文法过滤（不可注入换行），且仅返回给提交该值的调用方；daemon 不记录该错误详情。
- **跨仓库**：`pact-ref` vendored 快照已同步（`scripts/sync-spec.sh --check` 通过），`fmt`/`clippy`/`cargo test` **46 passed / 0 failed**。
- **未做（已入 `todo.md`）**：归档钉版与 `digestPolicy` 运行期强制、`onUnknownScheme`、`RefRule.value`、`declarationDigest` 漂移检测、WASM 入口校验、conformance 用例（runner 仍是骨架）。
- **文档**：新增 FR30、D45 与 §9.12；同步 §10 测试策略、`docs/manual.md`、`docs/thingref-governance.md`、`docs/architecture.md`；新建 `todo.md` 并在 `AGENTS.md` 建立进度台账纪律。

## 2026-10-28（第 76 轮，ThingRef 未尽事宜审视）

- **范围**：对照 `docs/thingref-governance.md` §3/§4/§8/§13、FR27–FR29、D41–D44，核对 `pact` workspace 与 `pact-ref` 当前实现；只读审视，未改代码。
- **已复核完成**：规范层（URI/Identifier/`∅` 分类、RefBindings 匹配、ContractCard、Tombstone、retention）；离线 lint（绑定/契约/声明/tombstone/可发现性）；视图可见性（`read-gate`/`domain`/`hidden`）；视图级读审计；快照同步（`pact-ref` 44 项测试通过）。
- **发现 1（最高优先级）：入口票面校验缺失。** `POST /seeds` 与 `POST /tokens` 的 `thing_ref` 直接透传为 `ThingRef`（`dto.rs` 的 `Option<String>` + `server.rs` 的 `seed_inner`/`produce_inner`），`pact-cli` 在线命令与 `pact-mcp` 同样只转 REST；`ReferenceForm::classify` 与 `RefBindings::resolve` 目前只在 `GET /counter`/`GET /archive` 的可见性计算与离线 lint 中调用。后果：userinfo、裸路径、控制字符、超长引用、Identifier 语法、`required`/`allowIdentifier`/`schemes`/`maxLength`/`onUnknownScheme` 均不在写入时拒绝。`docs/thingref-governance.md` §8.1 表的“入口校验 / `--thing-ref` 校验 / 与 REST 同一套校验”目前是目标而非现状。
- **发现 2：缺失与 `null` 未区分。** `SeedReq`/`ProduceReq` 的 `#[serde(default)] Option<String>` 把“字段缺失”和“显式 null”都折叠为 `None`，违反 `spec/ref/v1/uri-profile.md` §1 的“缺失 → 400，`null` → `∅`”。
- **发现 3：绑定策略与归档钉版未进入运行期。** `RefBindings` 的 `required`、`allowIdentifier`、`schemes`、`maxLength`、`onUnknownScheme` 只被 lint 或视图可见性间接使用；`pinning.requiredForArchive` 与 `digestPolicy=required-at-archive` 只在 lint 检查，`/archive` 不校验摘要（`uri-profile.md` §4 的归档钉版约束尚未落地）。
- **发现 4：规范层两处与 v1 profile 不一致（已用临时程序验证）。** `git+ssh://git@example.com/...` 是 `uri-profile.md` §3 的合法示例，却被 `has_userinfo` 以 authority 含 `@` 判为 `UserInfoNotAllowed`；Identifier 语法 `1*(ALPHA/DIGIT/_/-/./:/@/#)` 未执行，`foo bar`、`foo?bar` 被接受为 Identifier。
- **发现 5：RefBindings 存在未生效字段。** `RefRule.value`（`none`/`uri`/`identifier`）在 schema 与规范 crate 中保留但无消费点；`declarationDigest` 只检查“是否记录”，不参与匹配或漂移检测（后者 FR27 已声明为边界）；`onUnknownScheme` 只影响 lint 告警，不影响运行期。
- **发现 6：治理平面仍是骨架。** `pact-ref` 5 个 adapter 与 auth/invoker/registry/audit 均为占位，`refd` 只打印装配状态并注明 “No endpoints are wired yet”；resolve/describe/invoke/revoke 端点未实现，动态只读/动态写/可选注册表三个阶段未开始。
- **发现 7：其他入口形态未接入静态校验。** `pact-wasm` 直接构造 `ThingRef`，未做 URI/绑定校验；`pact-runtime` 无 resolve/invoke 审计边车。二者在治理文档中属“可支撑/适合放”的方向，不是已实现能力。
- **文档影响**：`docs/thingref-governance.md` §13.1 第 1 阶段补记“入口校验”待做；§8.1 表增加现状说明。未分配新 FR/设计编号；若把入口校验立项，编号须先在 `docs/architecture.md` 附录 A 登记。
- **建议顺序**：先定“入口校验放在 REST 边界还是引擎前”的边界（推荐 REST，CLI/MCP/runtime 自动继承）；再修 `git+ssh` 与 Identifier 语法两个确定性缺陷；随后做缺失/null 区分与绑定策略强制；最后才进入 `refd` 与动态阶段。

## 2026-10-28（第 75 轮，可发现性 lint 推送前安全审视）

- **范围**：第 74 轮改动（`pact-cli` 可发现性 lint、e2e、六份文档），提交 `eb86654`。
- **扫描结论**：提交内容仅 8 个文件、460 行新增；`Cargo.toml`/`Cargo.lock` 无变化；无敏感文件名、无 PEM 私钥或云凭据模式；新增生产代码无 `unsafe`、无网络/进程/文件写入、无 `unwrap`/`expect`/`panic!`（`unwrap` 仅出现在 e2e 测试）。
- **输入与失败模式**：`ReferenceForm::classify` 与 `RefBindings::resolve` 的失败分支均以 `let ... else { continue }` 跳过，不 panic；lint 只读用户显式传入的本地文件，不写盘、不访问网络；输出只含类型名、契约 id、scheme 与本地路径，**不输出 `thing_ref` 值**；e2e 只在 `tempfile` 目录写测试输入。
- **依赖审计**：`cargo audit --file Cargo.lock` 为 **0 vulnerability、2 unmaintained**（`unsafe-libyaml` RUSTSEC-2024-0449、`rustls-pemfile` RUSTSEC-2025-0134），与第 73 轮一致，无新增漏洞。
- **误报说明**：全仓凭据扫描时 `web/authoring/index.html` 的内嵌 WASM base64 触发 `sk-…` 模式，经人工确认为构建产物而非凭据。
- **接受的低风险项（沿用）**：RefBindings/契约/声明文件大小上限仍由运维控制；CI action 仍按主版本标签固定；`refd` resolve/describe 端点未实现。
- **推送**：`pact` `750c303..eb86654` → `origin/main`；`pact-ref` 本轮无代码改动（`7044f40`，工作区干净，vendored 快照 `sync-spec.sh --check` 通过）。
- **验证**：`cargo fmt --check`、`clippy -D warnings`、`cargo test --workspace --locked` **309 passed / 0 failed**。

## 2026-10-28（第 74 轮，引用可发现性 lint）

- **触发**：保留阶段收尾——D44 已有 retention/tombstone，“下游到底能不能拿到票面与保留证据”仍缺静态验收。
- **实现**：`pact ref lint` 对 `required=true` 的绑定新增 5 个 warning code（不新增命令与引擎能力）：
  - `ref.discoverability.required_hidden`：effective `visibility=hidden`，视图永不返回票面；
  - `ref.discoverability.view_read_path_missing`：effective read-gate，但声明中没有对该类型的**无条件** `reads`/`consumes`；
  - `ref.discoverability.no_reader`：声明中完全没有该类型的 `reads`/`consumes`（条件子句静态视为“可能成立”）；
  - `ref.discoverability.retention_missing`：被要求的契约无 `lifecycle` 且非归档钉版（归档钉版由 `ref.contract.retention_missing` 覆盖，避免重复告警）；
  - `ref.discoverability.deletion_without_tombstone`：被要求的契约 `deletionPolicy=delete`，到期后下游无法区分删除与故障。
- **判定细节**：scheme 样本取“绑定可见 scheme ∪ 契约声明 scheme ∪ 全局默认”，逐个用 `{scheme}://lint.invalid/ref` 合成票面走 `RefBindings::resolve`，任一 scheme 解析出 `required=true` 即视为要求引用；实例级只在实例显式声明该类型时检查，避免与类型级重复；`visibility=domain` 表示视图直接暴露票面，抑制读取路径类告警。
- **代码结构**：`ref_lint.rs` 新增 `check_discoverability`、`has_view_read_path`、`has_any_reader`、`check_required_contracts`；循环内不变参数收敛为 `DiscoverabilityCtx`，避免触发 clippy `too_many_arguments`。`check_bindings` 改为保留解析出的 `Domain`，同时供类型存在性校验与读取路径判定。
- **测试**：`pact-cli` 新增 2 项单元测试（可见性缺口、保留证据缺口）与 1 项 e2e（真实二进制 + JSON + `--strict` 退出码）；全 workspace **309 passed / 0 failed**，`cargo fmt`、`clippy -D warnings` 通过。
- **文档**：`docs/requirements.md` FR27/FR29 同步；`docs/design.md` D44 结论、§9.11 与 §10 测试策略同步；`docs/manual.md` §7.8/§7.10 增补；`docs/thingref-governance.md` §9.2/§13.1/§13.3 标记保留阶段完成。
- **边界（不变）**：只证明静态路径存在，不保证运行期一定可读；不校验内容是否已删除、tombstone 是否已发布；不做保留计时、自动删除或自动 tombstone。

## 2026-10-27（第 73 轮，pact CI 依赖安全审计）

- **触发**：第 72 轮安全审视把“`pact` CI 尚无 cargo audit 作业”列为剩余项，用户要求补齐。
- **变更**：`.github/workflows/ci.yml` 新增 `audit` job，固定 `cargo-audit 0.22.2`，缓存 `~/.cargo/bin/cargo-audit`，执行 `cargo audit --file Cargo.lock`。vulnerability 即失败（退出非零）；unmaintained/unsound 告警保持可见但不阻断。
- **顺带修复**：首次审计暴露 `tokio 1.53.1` 的 RUSTSEC-2026-0386（unsound：`AsyncFd` 构造器违反 I/O safety，patched ≥1.53.3）。项目未直接使用 `AsyncFd`，仍升级 `tokio` 至 `1.54.1`（仅 `Cargo.lock`）。审计现为 **0 vulnerability、2 unmaintained 告警**：`rustls-pemfile`（RUSTSEC-2025-0134）与 `unsafe-libyaml`（RUSTSEC-2024-0449）。
- **安全边界**：workflow 仍只有 `contents: read`，不创建 check run、不申请 `checks`/`issues` 写权限；因此 `pull_request`（含 fork，token 只读）不会因权限失败。未新增第三方 action，仅复用既有 `actions/cache@v4`。
- **验证**：workflow YAML 解析通过；`cargo-audit 0.22.2` 发行包自带 `Cargo.lock`，`--locked` 可复现安装；本地审计 0 vulnerability；`cargo fmt`、`clippy -D warnings`、`cargo test --workspace --locked` **306 passed / 0 failed**。
- **后续**：评估 `serde_yaml` → safe-Rust YAML 解析器、`rustls-pemfile` → `rustls-pki-types` 的替代，消除剩余 unmaintained 告警。
- **文档**：`docs/design.md` §10 测试策略新增“依赖安全审计”行。

## 2026-10-27（第 72 轮，推送前安全审视）

- **范围**：审查第 67–71 轮与任务即制品专题的待推送改动，覆盖 `pact-ref-spec`、`pact ref lint`、可见性过滤、视图审计、保留/tombstone、daemon 配置与 CI。
- **修复**：`Tombstone` 时间戳解析原用固定字节下标切片，含多字节字符的非法输入（如 `2026-10-27T00:00:0éZ`）会触发 char-boundary panic。改用 `str::get`，越界或非字符边界一律返回校验错误；新增回归测试 `rejects_non_ascii_timestamp_without_panicking`。
- **复核结论**：无凭据/密钥入库；视图审计只写类型计数与类型过滤器，不写 `thing_ref` 值；可见性加载失败与审计写失败均 fail-closed；新增代码无 `unsafe`；规范 crate 无 IO/网络；四份 schema 均 `additionalProperties: false`。
- **接受的低风险项**：RefBindings/tombstone 文件只在启动或离线 lint 阶段读取、路径由运维配置控制，未设文件大小上限；`serde_yaml` 及其底层 `unsafe-libyaml` 已停止维护（RUSTSEC-2024-0449 等为信息性告警，非已知漏洞），后续可评估替代；`pact-ref` CI 的 action 按主版本标签固定，建议后续改为提交 SHA。
- **验证**：`pact` 306 passed / 0 failed，`pact-ref` 44 passed / 0 failed；两仓库 fmt 与 clippy `-D warnings` 通过。
- **推送**：`pact` 推送 `origin/main`（`6342063`）；`pact-ref` 新建公开仓库 `https://github.com/onlookor/pact-ref` 并推送 `main`（`a4de714`）。
- **推送后 CI 修复**：首次公开推送的 Security 作业失败并非代码问题——`audit-check` 报 `Resource not accessible by integration`，原因是 workflow 仅有 `contents: read`、无法创建 check run（审计结果本身为 0 vulnerability、1 unmaintained）。已给 audit 作业补 `checks: write` 与 `issues: write` 并重跑。

## 2026-10-27（第 71 轮，引用保留与 tombstone）

- **触发**：推进物引治理路线的保留阶段。
- **生命周期收紧**：`Lifecycle.deletion_policy` 由自由字符串改为 `DeletionPolicy` 枚举；`retention` 只接受 `permanent` 或正 ISO 8601 duration（自研纯函数校验，周与日期分量互斥、分量按序、总时长 > 0）；`tombstoneRef` 仅在 `tombstone-then-delete` 允许且必须是 URI；归档钉版契约禁止 `delete`。
- **Tombstone**：新增 `pact-ref-spec` 的 `Tombstone` / `TombstoneMetadata` / `TombstoneSpec` / `TombstoneReason`，校验 `metadata.id` URI、契约 id、引用形态、RFC3339 UTC 时间戳与 `retainUntil > deletedAt`、`reason=replaced` 必填 `replacedBy`、`replacedBy != ref`、`note` 长度与控制字符；新增 `spec/ref/v1/tombstone.schema.json` 与 `SpecError::InvalidTombstone`。
- **Schema**：`contract-card.schema.json` 的 `deletionPolicy` 改为枚举，`retention` 增加 ISO duration 结构约束，`tombstoneRef` 用 `if/then` 与删除策略联动。
- **Lint**：新增 `--tombstones` 与 `LintReport.tombstones`；交叉校验契约存在性、删除策略一致性、tombstone 引用 scheme/长度、重复 id 与重复 (contract, ref)；归档钉版契约无 `lifecycle` 时警告 `ref.contract.retention_missing`。人类摘要与 JSON 输出同步。
- **边界**：不做保留计时、自动删除、tombstone 发布或解析可达性验证；引擎/存储语义不变。
- **验证**：`pact-ref-spec` 新增 9 项（retention/lifecycle/tombstone），`pact-cli` 新增 6 项单元与 e2e 覆盖；全 workspace fmt/clippy/test 通过。

## 2026-10-27（第 70 轮，视图级读审计）

- **触发**：继续推进物引治理路线，补齐 D42 遗留的视图级读审计。
- **审计 schema**：`pact-store::ReadAuditRecord` 新增 `kind`（`read` 缺省 / `view`）、可选 `view`、`type_filter` 与 `types[{type,tokens,refs_visible,refs_filtered}]`；`participant`/`token_id` 改为 `Option`，旧记录无 `kind` 按 `read` 解析。
- **写路径**：新增 `Engine::record_view_audit` 与 `Command::RecordViewAudit`。视图审计仍由引擎 actor 单写者落盘；`read_audit = false` 时无操作，开启时写失败上抛 `EngineError::Store`。
- **接口层**：Counter/Archive 过滤票面时同步按类型累计 `tokens/refs_visible/refs_filtered`，随后调用 `record_view_audit`；失败映射 `500`，视图不返回（fail-closed）。审计只记计数，不记 `thing_ref` 值。
- **边界**：审计仍不进入事件日志 / `seq` / 重放 / Rule-T / 快照 / 压缩；不改变 D42 的可见性判定。
- **验证**：`pact-store` 4 项 schema/审计流测试、`pact-engine` 2 项视图审计测试（字段/可选/fail-closed）、`pact-http` 2 项端到端（按类型计数、匿名与 fail-closed）通过；既有 daemon/engine 审计断言更新为可选字段。

## 2026-10-27（第 69 轮，物引可见性过滤）

- **触发**：继续推进物引治理路线，进入可见性阶段（FR28）。
- **新增模块**：`crates/pact-http/src/visibility.rs`：`VisibilityProfiles::load` 读取 RefBindings（文件/目录），`exposure` 按 `defaults → scheme → type → instance` 计算 `Visible / ReadGate / Hidden`。
- **接口层**：Counter/Archive 逐 token 过滤 `thing_ref`，token 存在性不变；`read-gate` 用 `engine.my_gates` 的无条件 `reads`/`consumes`，`produces` 不授予可见性。
- **身份**：令牌/签名模式下视图读取要求身份，匿名 `401`；信任模式无主体，read-gate 默认隐藏。
- **配置**：新增 `[ref_bindings].paths`；空 paths 拒绝启动；RefBindings 读取/解析/校验/重复域失败拒绝启动。
- **边界**：`GET /tokens/{id}` 仍由 Rule-R 授权；视图级读审计未实现（需要扩展审计 schema），列为后续。
- **验证**：`pact-http` 7 项 visibility 单测 + 3 项 visibility 端到端（read-gate/domain/hidden、匿名 401、Archive、信任模式）+ 1 项 daemon fail-closed 启动测试通过。

## 2026-10-27（第 68 轮，物引静态检查命令）

- **触发**：用户要求实现 `pact ref lint` 静态检查命令（FR27）。
- **新增模块**：`crates/pact-cli/src/ref_lint.rs`，纯函数入口 `lint_texts` + 路径入口 `lint_paths`；CLI 子命令 `pact ref lint`。
- **检查规则**：解析/规范校验、重复域与重复契约、绑定类型与 `.pact` 声明对照、契约引用可解析、绑定接受 scheme 与契约 scheme 一致（越界报错、收窄告警）、`required` 无契约、Identifier 放行、未注册 scheme 放行、`declarationDigest` 缺失、契约 retired/deprecated、未使用契约。
- **输出**：JSON 报告 `{bindings,contracts,errors,warnings,findings[]}`；人类模式逐条 + 摘要；有 error 退出 1，`--strict` 下 warning 也退出 1。
- **边界**：不解析具体 `ThingRef` 存在性、不访问网络、不进入引擎；不校验 `declarationDigest` 内容。
- **依赖**：`pact-cli` 新增 `pact-ref-spec` 与 `serde_yaml`；不新增 crate。
- **验证**：`pact-cli` 8 项单元测试 + 端到端测试（一致输入、未知类型、缺失/未使用契约、scheme 冲突、Identifier 放行、重复/退役契约、坏 YAML、缺失文件、`--strict`）通过；对 `pact-ref/examples` 实跑得到合理结论。

## 2026-10-26（第 67 轮，物引规范层抽取与消费）

- **触发**：用户要求先抽取 `pact-ref-spec` 并让治理项目切换消费版本。
- **新增 crate**：`crates/pact-ref-spec`（纯、无 IO、可 wasm32），定义 `ReferenceForm`（URI/Identifier/`∅`）、URI profile 校验、`RefBindings` 的 `defaults → scheme → type → instance/type` 继承匹配、`ContractCard` 静态/动态校验；不依赖 engine/store。
- **新增规范文件**：`spec/ref/v1/uri-profile.md`、`ref-bindings.schema.json`、`contract-card.schema.json`、`reference-plane.openapi.yaml`。
- **消费方式**：`pact-ref` 的 `ref-core` 改为 re-export 规范类型；`Cargo.toml` 使用 `version = "0.1.0"` + `vendor/pact-ref-spec` 只读快照。`scripts/sync-spec.sh --source <dir>` 同步，`--check` 检查漂移。选择快照而非跨仓库 path，避免 CI/容器构建检出另一仓库；注册表发布后删除快照、只留 version。
- **示例对齐**：`pact-ref/examples` 的截断摘要替换为合法 64 位十六进制；新增集成测试把真实示例解析为规范类型并校验。
- **未做**：`pact ref lint` 尚未实现；规范 crate 尚未发布到包注册表；真实适配器与 `refd` 端点仍未实现。
- **验证**：`pact-ref-spec` 18 项单测、clippy、`wasm32-unknown-unknown` 构建通过；`pact-ref` workspace 35 项测试、clippy、`refd` 引导输出通过；两份 JSON schema 与 OpenAPI YAML 解析通过。

## 2026-10-26（第 66 轮，用户向文档定位重写）

- **触发**：外部对话显示，README / 手册以内部术语（“不重写 loop”“治理闸门”）开场，读者无法判断 PACT 属于哪类工具，以及它与 LangGraph / Temporal / BPMN / agent 框架等的关系。
- **README**：开头改为“声明式协同层”一句话 + 它做什么 / 不做什么；新增 “Where it fits / 归入哪一类” 与八类常见工具的对比表（图式 agent 编排、持久化执行、BPMN、DAG 调度、agent 框架、Harness 内团队 / 任务板、策略引擎、Petri 网 / CPN）；功能列表按“建模与检查 / 运行与治理 / 接入与运维”重组；新增 DSH 适配器段落，明确 PACT 独立于 harness。
- **手册**：§1 重排为 1.1 一句话定位 → 1.2 归类与对比（含 Harness 内团队 / 任务板）→ 1.3 它不是什么 → 1.4 六原语 → 1.5 接入形态 → 1.6 适用 → 1.7 边界；§13 重写 DSH 插件定位，明确 `govern` 是准入决策外移、域声明即流程定义，并给出 loop / graph / team / PACT 的分层关系。
- **examples / 契约**：两份 `examples/README` 增加定位段落，`cross_harness` 说明改为“无中心*任务*编排器，PACT 引擎是共享准入权威”；`authoring-contract.md` 引言增加定位句。
- **交叉引用**：`docs/expressiveness-boundaries.md` 与 `docs/token-consumption-timing.md` 的手册 §1.5 引用改为 §1.7；手册跨实例限制的 §1.3 引用改为 §4.7；手册引用约定新增 `手册 §x.y` 前缀。
- **验证**：保留 `#what-is-pact` / `#pact-是什么` / `#testing` / `#测试` 锚点，新增 `#where-it-fits` / `#归入哪一类` 并在 examples 中引用；相对链接、锚点与表格列数已脚本检查；代码与行为零改动，未重跑测试（既有全量记录为 245 项）。

## 2026-10-25（第 65 轮，文档编号清理）

- **触发**：用户要求集中清理对话中自创、文档中无解释使用的编号（`L0–L4`、`P1–P9` 等）。
- **消费时机引用**：§18 表格中裸 `D1–D6 / P1–P9 / A1–A9` 改为 `TC-D1–TC-D6 / TC-P1–TC-P9 / TC-A1–TC-A9`。
- **WASM 命名空间**：`S2` → `W-S2`；耦合说明中的 `G1/D27` → `D12/D27`。
- **自驱策略**：阶段 5/7 描述中的 `P1/P2/P3` 改为“生产优先 / 汇消费 / 等待三档”，不再使用编号。
- **架构分层标签**：§13 的 `L1/L2/L3` 改为描述性名称（范式引擎层 / 领域适配层 / 生产运行层），与 `architecture.md` 附录 A 一致。
- **代码与行为零改动**，全量测试仍为 245 项。

## 2026-10-25（第 64 轮，控制面认证、域级授权与审计）

- **#3 认证边界**：新增 `control_principal` / `authorize_control_key` / `authorize_control_wildcard` 及对应检查方法；配置任一身份（Bearer 或签名）后，`handoff` / `freeze` / `GET|PUT /active` 必须可认证；信任声明模式（两者皆空）仍匿名放行。既有生命周期端点不套用此规则。
- **#4 交接授权**：`POST /domains/{d}/handoff` 在 ACL 下要求源实例键、目标实例键与域通配键 `{d}/*`——交接会改写域级 `declaration.pact`，仅实例键不足以覆盖该副作用。身份只解析一次，三项检查复用同一 principal。
- **#5 审计**：`write_sealed` / `write_active` 增加 `ts`/`by`；`SealedMarker` 改为 JSON（`{version, reason, ts, by}`），`ActivePointer` 升至版本 2。`Clock::now()` 为唯一时间源，交接中冻结标记、`Imported` 与活动指针共用同一时刻。新增引擎命令/方法 `ActivePointer` / `active_pointer`；`ActiveResp` 增加可选 `ts`/`by`。旧版文件缺字段按 `None` 读取。
- **兼容**：`Engine::handoff` / `set_frozen` / `activate` 保持原签名（委托 `_as` 变体，`actor = None`），CLI/内部调用零改动。
- **#11 文档**：`mcp.rs` 模块说明由“缺省 `default`”更正为“缺省活动实例，无指针时为 `default`”；手册 §2.3/§4.7/§7.6/端点表同步审计、认证与 ACL 语义（#6）。
- **测试**：`daemon_e2e` 新增 `control_endpoints_require_identity_and_record_audit`；`handoff_acl_requires_source_target_and_domain_wildcard_keys` 验证“源+目标键齐备但缺通配键 → 403”。全量 245 项通过。

## 2026-10-25（第 63 轮，冻结完整性修复）

- **问题**：`instantiate` 固定以 `frozen: false` 建会话且不读 `.sealed`；冻结源被 `unload` 后经重载/实例化即可写入，而磁盘标记仍在，重启后又变回冻结。
- **修复**：`instantiate` 在建会话前检查 `.sealed`，存在即 `InstanceFrozen`；`load_domain` 的 `Some` 空实例分支同样拒绝，`None`（重载）分支仅在 `force` 时清除标记后重建；`handoff` 目标存在性检查纳入 `.sealed`（`InstanceExists`）。
- **语义边界**：`force` 重置仍是唯一的显式解冻路径（与 §8.6 表格一致）；冻结继续定位为运维护栏而非 ACL 级防篡改边界。
- **测试**：`crates/pact-engine/tests/handoff.rs` 新增 `frozen_source_rejects_reinstantiation_without_force`、`handoff_rejects_target_with_stale_sealed_marker`；全量 244 项通过。

## 2026-10-25（第 62 轮，移除讨论阶段方案编号）

- **动机**：讨论期使用的临时方案编号只在对话中成立、仓库文档从未定义，且与 `docs/authoring-contract.md` 已有的 H1–H5「硬停」编号冲突。正式文档不应依赖未定义的临时约定，故全局移除。
- **处置**：全局取消这些编号，改用描述性表述（“显式状态交接”“活动实例别名”“原位换版”“外部 watcher”）。涉及 `docs/requirements.md`、`docs/design.md`、`docs/architecture.md`、`docs/manual.md` 及三份 change 文档、`README.md`、代码注释与错误原因字符串、`.trae/specs/state-handoff/`。
- **命名**：集成测试文件 `crates/pact-engine/tests/h3c_handoff.rs` → `handoff.rs`；测试名 `h3c_handoff_and_alias_over_unix_socket` → `state_handoff_and_alias_over_unix_socket`、`mcp_h3c_tools_over_stdio` → `mcp_state_handoff_tools_over_stdio`；spec 目录 `h3c-handoff` → `state-handoff`。
- **顺带修正**：§9.1 `pact_domain_freeze` 参数名改为实际的 `unfreeze`；§9.2 冻结请求体 `{freeze}` → `{frozen}`；读审计短路由说明由“default 实例”改为“按活动实例解析”。
- **未改**：`docs/authoring-contract.md` 与作者页中既有的 H1–H5 硬停编号保持原样；`paper/pactv1_rev3.md` 零改动。
- **验证**：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（242 项）全绿。

## 2026-10-25（第 61 轮，D39 显式状态交接与活动实例别名）

- **动机**：长周期协作流需要在换版时不丢失已积累的 Counter/Archive，且客户端不应被迫改路由；外部 watcher 只解决声明更新、原位保留状态的声明换版改动面过大，取显式状态交接 + 域级活动实例别名。
- **模型**：新增 `Event::Imported`（自足基线，携带完整 `CounterState`/幂等表/源域/源实例/源 seq/目标声明），写 schema `version = 2`；`SUPPORTED_EVENT_SCHEMA_VERSION = 2`，更高版本 fail-fast。恢复取最后一个基线（`DomainLoaded` 或 `Imported`），`is_baseline` / `baseline_declaration` 统一两者。
- **存储**：新增 `<domain>/active.json`（`{version, instance}`，原子写、损坏 fail-fast）与 `<domain>/<instance>.sealed` 冻结标记；索引与 `rebuild` 投影 `Imported` 的 Counter/Archive。
- **引擎**：`handoff` 顺序为 校验名字/目标不存在/解析声明/类型闭包 → 冻结源 → 追加 `Imported`（提交点）→ 建目标会话 → 更新声明注册表 → best-effort 激活；追加失败回滚冻结。新增 `InstanceFrozen` / `HandoffIncompatible`；Seed/Produce/新 Consume 拒绝冻结，幂等重放放行；恢复校验活动指针，悬空 fail-fast；`unload_domain` 清除指针。
- **接口**：REST `POST /domains/{d}/handoff`、`GET/PUT /domains/{d}/active`、`PUT /domains/{d}/instances/{i}/freeze`；短路由经 `resolve_key` 解析活动实例，长路由显式；交接 ACL 同时校验源/目标键，激活要求 `domain/*`；MCP 工具 17 → 21；CLI 增四命令。
- **客户端语义收窄**：`base_path` 仅在 `instance = None` 时使用短路由，`Some("default")` 改为显式实例路由，保证别名启用后仍可显式访问源 `default`。
- **回滚边界**：交接提交后不回滚；源保持只读，可人工解冻或继续以长路由访问；激活失败只反映在 `activated=false`。
- **未改论文**：`paper/pactv1_rev3.md` 零改动；形式语义按实例成立，实例划分仍属实现自由。

## 2026-10-25（第 60 轮，README 导览）

- **范围**：只改 `README.md` / `README.zh-CN.md` / `docs/manual.md` 的组织方式，不改代码、不改页面、不改引擎。
- **导览**：两份 README 在“功能特性”之后、“快速开始”之前新增按用户与场景分流的表格（判断适用性、agent 内写域、无工具写域、手写域、运行与驱动 agent、部署运维、消费时机裁决、贡献代码），每行给出“先看 → 然后”，并与 L0–L4 阶梯、在线地址、手册 §12 互链。
- **整合**：原先夹在“快速开始”里的大段引用块（契约 / skill / 三个页面 / 在线地址）并入导览表格；“快速开始”只保留构建、两条示例脚本与 `pact analyze` 命令。`examples/README.md` 的五域、两片段、两 demo 说明收进导览末尾一句，避免与表格重复。手册 §12 增加一行指回 README 导览，形成双向引用。
- **验证**：本地相对链接逐项存在；GitHub 锚点 `#what-is-pact` / `#testing` / `#pact-是什么` / `#测试` 与标题一致；表格列数一致。

## 2026-10-25（第 59 轮，发布前安全审视与 Pages）

- **范围**：不改引擎、语法、母本与既有端点。新增静态入口 `web/index.html` 与发布流水线 `.github/workflows/pages.yml`；把 L0–L4 的资产一起发布。
- **入口页**：`web/index.html` 是纯静态的三步入口（写域 → 跑一遍 → 接 agent），与三个页面同源配色、`≤760px` / `≤480px` 两档断点，含嵌入说明与在线地址。不含占位符，不参与 `web/build.sh`。
- **发布**：`Pages` workflow 在 `main` 上 `web/**` 或 workflow 自身变化时触发（也可手动）。只把 `web/index.html` 与三个 `index.html` 复制到 `_site` 后 `upload-pages-artifact` → `deploy-pages`；**不在 CI 重跑 wasm 构建**，发布内容与仓库内可双击的文件逐字节一致。权限收窄为 `contents: read`、`pages: write`、`id-token: write`，并发组 `pages` 取消进行中的旧发布。
- **发布前验证**：
  - `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿（含 `pact-wasm` 18 项）。
  - 构建可复现：`./web/build.sh` 重跑后三页 SHA-256 与重建前一致。
- **安全审视**：
  - 三页无 `<script src>` / `<link href>` / `<img src>` / `<iframe src>` 外链，无 `fetch` 调用（glue 中的 `fetch` 分支只在传入字符串/URL 时触发，页面传 `Uint8Array` 字节），无 `eval` / `document.write` / `insertAdjacentHTML`。
  - 所有动态 DOM 文本经 `esc()`（含 `& < > " '`）；生成配置一律写入 `textContent`；契约正文以 `text/plain` 注入且构建校验 `</script`。
  - 接入向导的令牌由 `crypto.getRandomValues` 本地生成，不落盘、不请求网络；页面不含任何真实密钥。
  - 仓库改动无凭据/私钥/令牌；发布只暴露三个单文件页面与入口页，不发布模板、构建脚本或契约以外的文件。
- **文档**：`docs/design.md` 新增 D38；`docs/manual.md` §3.5 增在线入口与构建说明；`README.md` / `README.zh-CN.md` 增浏览器路径与在线地址。

## 2026-10-25（第 58 轮，L4 接入向导）

- **动机**：域能写（L0–L2）、能跑（L3）之后，最后一道门槛是把域接到 agent：手写 `pact.toml`、判断 stdio 还是 HTTP、拼 MCP 客户端配置，任一步错都表现为“连不上”。
- **新增** `web/connect/`：纯静态生成器（18 KiB，不依赖 wasm），三块产物 + 一份顺序说明：
  1. `pact.toml`——stdio 模式只给 `data_dir` / `listen_unix`；HTTP 模式追加 `listen_tcp`、`[mcp] enabled/public_url` 与 `[auth.tokens]`；
  2. MCP 客户端 JSON——stdio 用 `command`/`args --unix`；HTTP 用 `type: http`、`url …/mcp`、`Authorization: Bearer`，并注明字段名随宿主而异；
  3. 命令序列——`analyze` → `daemon start --config` → `domain load` → `domain list/status`，统一带 `--unix` 与（HTTP 模式的）`--bearer`；
  4. “接上之后”提示——先 `pact_domain_validate` 再 `pact_domain_load`，用 `pact_my_gates` / `pact_enabled` 自省。
- **正确性来源**：参数逐项取自真实 `--help`（`pact`、`pact status`、`pact daemon start`、`pact-mcp`）；域名的权威来源是文件 stem，因此改域名会同步改文件名，避免 `status --domain` 与实际装载名不一致。令牌用 `crypto.getRandomValues` 现场生成（48 hex），并对短令牌、非回环明文 HTTP、相对 socket、依赖 PATH 的 `command` 给出即时警告。
- **实测**：把页面生成的两种模式配置分别写入 `/tmp` 启动真实 daemon——stdio 配置 `/health` 返回 `{"status":"ok"}`；HTTP 配置同时监听 unix 与 `127.0.0.1:18787`，TCP `/health` 返回 200。说明生成的键名与 `deny_unknown_fields` 兼容。
- **构建**：`web/build.sh` 的页面表新增 `connect: {}`（无 wasm、无契约、无配方占位符），一次构建输出三页。
- **适配**：复用第 57 轮断点与 `?embed=1`。探针实测 connect 在 1280/768/480/375/320 下无横向溢出、embed 隐藏标题与页脚、`≤480px` 输入字号 16px、更宽 13px（修正了 `.field input` 的 `font: inherit` 特异性压过媒体查询的问题）。
- **文档**：`docs/manual.md` 新增 §3.7“接入向导（无需安装）”，§3.6 的“两个页面”改为“三个页面”；`README.zh-CN.md` 快速开始增第三入口。
- **范围**：只新增前端资产与构建表条目；不改 CLI、不改 daemon、不改 MCP 工具面。
- **状态**：工作树改动，未提交、未推送；GitHub Pages 发布仍与最终一批改动合并进行。

## 2026-10-25（第 57 轮，页面精简 · 多端适配 · 可嵌入）

- **范围**：只改 `web/authoring/template.html` 与 `web/sandbox/template.html` 的结构与样式，不改 wasm、不改引擎、不改分析逻辑。
- **精简**：作者页的“开始之前”（G1–G5、H1–H5、提示词、决策记录模板）由常驻大面板改为**默认收起**的折叠面板，只留一行摘要；编辑器最小高度 480 → 360；沙盒编辑框 200 → 160，动作区提示压到一句话（保留“前驱不被消费”“不可逆”等关键语义），消费行不再重复同一句说明。
- **多端适配**：两页统一两个断点——`≤760px` 收窄留白、隐藏副标题、压缩面板头；`≤480px` 进一步缩小圆角/间距、按钮加高到触摸友好尺寸、`select/input/textarea` 提升到 16px（避免 iOS 聚焦自动缩放）、编辑器降高。主栅格在 980px 以下已单列，`.panel` 全部 `min-width:0`，`html` 加 `-webkit-text-size-adjust:100%`，窄屏日志详情取消左缩进。
- **可嵌入**：`?embed=1` 时给 `<html>` 加 `.embed`：隐藏标题与副标题、页头只保留状态胶囊并右对齐、页脚隐藏、`main` 去掉 `max-width` 以铺满容器；外链（沙盒 → 作者页）加 `target="_blank" rel="noopener"`，避免在 iframe 内导航走失。页面仍无外部资源、无 `fetch`、无 `eval`，被第三方 iframe 引用时不会请求任何跨域内容。
- **验证**：
  - 同源 iframe 自检（临时工具页，不入库）：两个页面 × 7 个宽度（1280/1024/768/600/480/375/320）在 `?embed=1` 下，再加 1280/375 的非嵌入模式，共 18 组。结果：全部 `scrollWidth == clientWidth`（无横向溢出）；embed 组 `h1/footer` 均 `none`，非 embed 组均 `block`；`≤480px` 编辑器字号 16px、更宽为 13px；状态胶囊均正常。
  - 功能回归：沙盒 R1 的 seed `request` → produce `answer`（Counter 双 token、日志同步）；作者页展开“开始之前”、勾选 G1、载入 R1 并分析得到“良构 · 可装载”，折叠正常，console 无报错。
  - `./web/build.sh` 重新生成两页（1208 KiB / 1193 KiB）。
- **技能链接**：确认 `~/.trae-cn/skills/pact-domain-authoring` → 仓库 `skills/pact-domain-authoring` 的符号链接存在且可解析，本轮未改动。
- **状态**：工作树改动，未提交、未推送；GitHub Pages 发布仍与最终一批改动合并进行。

## 2026-10-25（第 56 轮，L3 浏览器运行沙盒）

- **动机**：域写出来之后，下一步是“跑一遍”才能形成直觉。安装 daemon 是最大的一道门槛，而 `pact-wasm` 已具备完整宿主（装载、两阶段提交、四类动作、视图、事件导出），不需要新增引擎能力。
- **新增** `web/sandbox/`：
  1. `template.html`——装载域（粘贴或六个示例）→ 以参与者身份 seed / produce / read / consume → 实时渲染 Counter、Archive、状态卡、事件日志、Gate 矩阵与前置图；被拒绝时显示错误码 + 原因 + 该码的语义提示；
  2. 配方下拉与工作台共享；默认载入 R1 线性流水线，装载即做良构检查（失败则列出违规并拒绝装载）。
- **`pact-wasm` 新 API**：`PactHost::facets(key) -> DomainFacets` 与 wasm 同名导出。返回 `participants`（gate 主语 ∪ seed 主语，排序去重）、`seeds`（`participant` + `type`，字段名与 Token 一致）、`preconditions`（`target` + `predecessors`）。此前浏览器端只能拿到按参与者的 gate 子句，无法知道“域里有哪些参与者、谁能 seed 什么”，被迫在 JS 里重写一遍 DSL 解析。新增单元测试 `facets_lists_participants_seeds_and_preconditions`（含线上字段名锁定）。
- **构建整合**：`web/authoring/build.sh` 与 `web/sandbox/build.sh` 合并为 `web/build.sh`——一次 wasm 构建，按页面表注入占位符，并检查残留 `__PLACEHOLDER__`；六个配方抽到 `web/shared/recipes.js`，两个页面共用。删除 `web/authoring/build.sh`。
- **安全边界**：沙盒**装载并运行**域，但只在页面内存中：无持久化、无网络、无外部脚本、无 `eval`；只有用户显式点击才产生一次 Blob 下载（事件日志 JSONL）。所有动态 DOM 文本经 `esc()`；wasm 仍由内联 base64 字节实例化。与工作台的差别写入 `docs/manual.md` §3.5 边界说明。
- **文档**：`docs/manual.md` 新增 §3.6“浏览器运行沙盒（无需安装）”，§3.5 的重建命令改为 `./web/build.sh`；`README.zh-CN.md` 快速开始增沙盒入口。
- **验证**：
  - `cargo fmt --all -- --check` 通过；`cargo test -p pact-wasm` 18 项（11 单元 + 7 parity）全过。
  - 浏览器实测（本地 HTTP）：沙盒默认载入 R1，`client` seed `request` → `responder` produce `answer`（Counter 同时保留 `request`，验证 Rule-P 不消费前驱）→ `client` consume `answer`（Archive 1、Counter 1）；R4 中 `coordinator` 在缺 `readiness` 时消费被拒（`GateConditionFailed` 条件标签 `exists readiness`），补 seed 后 produce `completion` 成功；`responder` 消费 `request` 后 Counter 归零、Rule-T 事件 `Terminated` 落一条 #seq，再 seed 即翻回未成立。console 无报错。
  - 工作台回归（配方抽取 + 重构建）：R2 六项全零、乱码报 `Syntax`、R6 全零，无 `ReferenceError`。
  - 语义澄清：装载后的初始状态 `terminated = true` 是正确的（Counter 为空且暂时无人可 Rule-P），页面改为显示“初始状态”文案而非告警式措辞；不是状态残留。
- **范围**：新增前端资产 + 一个只读视图 API；不改引擎语义、不改语法、不改母本、不改既有端点。GitHub Pages 部署仍未做（待与最终一批改动一起发布）。
- **状态**：工作树改动，未提交、未推送。

## 2026-10-25（第 55 轮，L2 浏览器单文件工作台）

- **动机**：L1 skill 依赖 agent harness；没有 harness、只有公开 chat 模型的用户需要一条“离线可验证”的路径，否则生成的 `.pact` 无法自证良构。
- **新增** `web/authoring/`：
  1. `template.html`——页面源模板，三处占位符 `__WASM_GLUE__` / `__WASM_BASE64__` / `__CONTRACT_TEXT__`；UI 含六配方下拉（R1–R6）、`.pact` 编辑器、Ctrl+Enter 分析、复制修复提示、下载 `.pact`、六张分析结果卡片、G1–G5 勾选、H1–H5 硬停、场景输入 + “复制给模型的提示词”、决策记录模板；
  2. `build.sh`——`cargo build -p pact-wasm --target wasm32-unknown-unknown` → `wasm-bindgen --target no-modules` → Python 内联 glue + base64 wasm + 契约正文，产物 `web/authoring/index.html`；
  3. `index.html`——构建产物，单文件约 1185 KiB（其中 wasm 826 KiB），双击即可离线使用。
- **分发形态**：不新增域格式，公开输入仍是 `.pact`；工作台只是把 `pact analyze` 同源的良构检查与六项分析搬到浏览器。无 `fetch`、无外部脚本、无 `eval`；wasm 由内联 base64 字节实例化；契约文本以 `text/plain` 注入，组装时校验其中不含 `</script`；所有动态 DOM 文本经 `esc()`。
- **`pact-wasm` 接口增强（行为变更）**：`FacadeError` 新增 `detail: Option<serde_json::Value>`（`skip_serializing_if = "Option::is_none"`），`Problem::WellFormedness` 时携带违规列表。此前浏览器端拿不到结构化违规，只能把良构失败误报为“解析失败”。为容纳 `Value`，`FacadeError` 的 derive 去掉 `Eq`、保留 `PartialEq`。新增单元测试 `wellformedness_error_carries_structured_detail`。
- **页面状态区分**：`WellFormedness` 分支显示“良构检查未通过：N 处违规”，不渲染六张卡片（A.5 是其余分析的前提），状态胶囊为“良构未通过 · 需修复”；语法错误仍走独立分支。修复提示在“有报告”与“仅有良构违规”两种状态下均可用。
- **L0/L1 口径修正**：契约与 skill 原文暗示“非良构也会输出六项报告，需逐项裁决”。实测为非良构时命令返回 `{"error":{"code":"WellFormedness","detail":[...]}}`、exit 1，**没有**六项报告；已改为“先清 violations，良构后才输出六项分析”，skill 的 DoD 同步改为“exit code = 0（不再返回 `WellFormedness`）”。
- **入口**：`docs/manual.md` 新增 §3.5“浏览器工作台（无需安装）”（含重建命令与“只做离线分析”边界）；`README.zh-CN.md` 快速开始增第三入口。
- **验证**：
  - `cargo fmt --all -- --check` 通过；`cargo clippy -p pact-wasm --all-targets -- -D warnings` 零警告；`cargo test -p pact-wasm` 17 项（10 单元 + 7 parity）全过；`cargo test --workspace` exit 0。
  - 浏览器实测（本地 HTTP）：状态“分析器就绪 · pact-wasm/0.2”；R1 六项全零、状态“良构 · 可装载”；坏域报 1 处 `ProduceWithCondition`、无六卡片、状态“良构未通过 · 需修复”；恢复 R1 后六项全零；console 无新增报错。
- **范围**：新增前端资产 + `pact-wasm` 错误明细字段；不改引擎语义、不改语法、不改母本。GitHub Pages 部署未做。
- **状态**：工作树改动，未提交、未推送。

## 2026-10-25（第 54 轮，L1 域作者 Skill）

- **动机**：L0 契约是“权威流程”，但 agent 需要可加载的执行器——负责工具探测、分批提问、错误映射与验收。
- **新增** `skills/pact-domain-authoring/SKILL.md`（`name: pact-domain-authoring`，v0.1.0）：
  1. **边界**：只做域建模/审查，不做运行、部署、引擎修改；
  2. **契约定位**：`PACT_CONTRACT` → 仓库 `docs/authoring-contract.md` → skill 内 `references/` 副本 → 问用户；未读契约不得起草；
  3. **硬约束**：先问后写、不编造、只产出 `.pact` + 决策记录、`produces` 无条件、源类型不可 produce、非零分析必须裁决、范式外能力不承诺；
  4. **状态机 S0–S7**：定位 → 适用闸门 G1–G5 → 硬停 H1–H5 → 分批采集（每轮 ≤3 问）→ 选配方 R1–R6 → 起草 → analyze 回环（≤5 轮，附 detail→修法映射）→ 交付；
  5. **完成标准**：`analyze` exit 0、violations 0、每处非零有裁决、决策记录完整、假设已确认、给出装载命令；
  6. 内置输出模板、常见失败模式、引用表。
- **反重复**：Skill 不复制契约正文，只做执行层；契约仍是唯一流程源。分发到仓库外时按 Skill §1 的分辨顺序随附 `references/authoring-contract.md`。
- **入口**：`README.zh-CN.md` 快速开始增 Skill 入口；`docs/manual.md` 既有契约入口不变。
- **验证**：frontmatter YAML 解析通过（name/description/version/tags）；Skill 引用的 5 个路径均存在；`pact analyze --json`、`pact --json analyze` 两种写法实测可用；二进制探测顺序在本仓库命中 `./target/debug/pact`（release 不存在）；按 Skill 流程试建复合场景（R2 只读评审 + R4 外部物化），六项分析全零、exit 0。
- **范围**：纯文档/资产，无代码、无测试、无母本改动。L2 浏览器向导待裁决。
- **状态**：工作树改动，未提交、未推送。

## 2026-10-25（第 53 轮，域作者契约 L0：降低建模门槛）

- **动机**：范式实现已足够复杂，无专业背景者难以从零写出合法域。用户裁决先落 L0（作者契约 + 形状配方库），公开输入格式仍保持 `.pact` 唯一。
- **新增** `docs/authoring-contract.md`：面向人与模型的生成协议，含
  1. 两种用法（有 `pact analyze` 回环 / 无工具自检）；
  2. 五条适用判据（G1–G5）与五个硬停（H1–H5，映射 B4/B10/B15/B11/B2）；
  3. 五步建模问题树（离散产物 → 前置图 → 权限矩阵 → 入口 → 逐类型消费裁决）；
  4. 六个形状配方（线性流水线、只读评审、`via` 审批、外部物化见证、ack 汇聚、循环点火），每个附实测 `analyze` 预期；
  5. 八类良构错误的“修法”列、六项分析解读与裁决原则、决策记录模板、12 条无工具自检清单。
- **核心口径**：表达力边界与消费时机不作为“知识灌输”，而作为**强制决策点**——B 类在前置闸门拦截或改写，TC 类在第五步逐类型逼问（Q1–Q7）。
- **修正** `docs/manual.md` §3.1 示例：原示例有 3 处良构违例（`UndeclaredType: security_report`、`ProduceWithCondition`、`SourceTypeProduceGate: test_report`），与 §3.3“八类拒绝”自相矛盾。改为合法示例并注明 `consumption_reachability: 1` 为预期（`implementation` 作 `via` 见证保留）。
- **入口引用**：`docs/manual.md` §3 开头、`examples/README.md` §7.1 增契约入口；契约内回链 manual §3、B、TC、examples。
- **验证**：`pact analyze` 实测 R1/R2/R3/R5（临时文件）、`examples/domains/external_materialization.pact`、`examples/domains/react_loop.pact` 与修正后的 manual 示例，结果与契约记载一致（R3 的 A.3=1、R6 的 SEED_PRODUCE_OVERLAP 为预期非零）。
- **范围**：纯文档，无代码、无测试、无母本改动。后续 L1（skill）与 L2（浏览器向导）待用户裁决后另开轮次。
- **状态**：工作树改动，未提交、未推送。

## 2026-10-24（第 52 轮，消费时机文档收紧 + 重复消费防护）

- **来源**：第 51 轮审查结论经用户裁决，处理三项：收紧 5 处文档表述、补 ack 屏障端到端测试、加固重复消费防护。
- **文档收紧**（`docs/token-consumption-timing.md`）：
  1. TC9/P8 的并发结论加宿主前提：原生由单写者 actor 串行化，WASM 宿主须保证同一实例 `prepare → commit` 间无其他写命令（`docs/wasm-local-spike-feasibility.md` §4.3），违反时由 `apply_event` fail-closed。
  2. “Archive 只作审计”改为“不参与 Rule-R/E；用于审计查询与 TokenId 唯一性守卫（`used_ids`）”（§2.2 补充段与 §7 表）。
  3. “保留任何 token 即不终止”限定为“Counter 上的 token”，并注明 Archive 不阻止终止（TC13、§7 表、`wait.rs` 注释）。
  4. D3 补准入时点：`count` 在准入时按当前 Counter 求值，若 `T = type(t)` 则目标自身计入阈值，消费后计数才减一。
  5. P7 明确引擎不内置 Archival Policy（对齐 `docs/manual.md` §3.2/§6），AP 等价于持 `consumes` gate 的参与者显式执行 Rule-E。
- **重复消费防护（行为变更，fail-closed）**：
  - `pact-store-model`：新增 `ModelError::ConsumeTargetNotOnCounter { token_id }`；`apply_event` 的 `Consumed` 分支先校验目标仍在 Counter，缺失即返回该错误（此前为“移除无操作 + 仍追加 Archive”，会重复归档并污染审计与 `used_ids`）。
  - `pact-store`：`StoreError` 同步新增同名变体与 `From<ModelError>` 映射、`Display`。
  - `pact-wasm`：`FacadeError::model` 映射新码 `ConsumeTargetNotOnCounter`。
  - 影响面：原生在线路径不受影响（`rule_e` 已在单写者临界区内校验）；WASM 两阶段宿主若违反串行契约，第二次 `commit` 返回错误且不落归档。日志重放侧，目标缺失视为日志/状态不一致，同样 fail-fast（压缩后的快照尾部重放仍满足“目标在 Counter”前提）。
- **测试**：
  - `pact-store-model`：`apply_event_consumed_rejects_absent_target`；原 `apply_event_consumed_is_self_contained` 调整为“先 seed 后消费”，仍验证归档内容完全来自事件。
  - `pact-wasm`：`duplicate_consume_commit_fails_closed`（两次不同 `request_id` 的 prepare 均可准入，第二次 commit 拒绝，Archive 仅一条）。
  - `pact-engine`：`ack_barrier_consumes_shared_after_all_downstreams`（屏障未满足拒绝消费 → 两下游各产 ack → 协调者消费共享 token → ack 不被级联删除 → 清理 ack 后触发 Rule-T）。
- **文档同步**：`docs/design.md` §7.4 事件不变量、§8.5 D23 归档原子性行、§11 阶段 3 验收措辞；`docs/manual.md` §9 错误码表 `Store` 行。
- **验证**：`cargo fmt --all -- --check` 通过；`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo test --workspace` **224 项全绿**（基线 221 + 3）。
- **状态**：已提交并推送至 `main`（提交 `959344b`）。

## 2026-10-24（第 51 轮，消费时机实现一致性审查）

- **范围**：对照母本 §1.6/§1.7.3 与 `docs/token-consumption-timing.md` TC1–TC13，检查 `rule_r`/`rule_p`/`rule_e`、`GateExpr::Via`/`Count`、`termination_holds`、`prepare_consume`/`commit`、`apply_event`、A.3/A.6。
- **结论**：核心语义一致，未发现必须改代码才能符合母本的行为缺口。Rule-P 不消费前驱；Rule-R 无状态；Rule-E 仅移除目标并归档；`via` 只查当前 Counter；`count` 在准入时按当前 Counter 求值；Rule-T 只要求 Counter 为空；Archive 不参与 Rule-R/E；A.3/A.6 与文档描述一致。
- **待收紧的文档表述（本轮未改，待用户决定）**：
  1. **TC9/P8 并发结论需限定宿主契约**：原生引擎由单写者 actor 串行化，WASM 两阶段宿主按 `docs/wasm-local-spike-feasibility.md` §4.3 要求同一实例 `prepare → commit` 间不得插入其他写命令。若宿主违反该契约，两次不同 `request_id` 的 `prepare_consume` 可先后通过并提交；`apply_event` 对已不在 Counter 的目标按“无操作”处理并仍追加 Archive，会产生重复归档。建议在 TC9/P8 注明该前提，或由宿主/`apply_event` 增加防护。
  2. **“Archive 只作审计”不完整**：`Session::recover` 以 Counter + Archive 重建 `used_ids`，Archive 同时承担 TokenId 唯一性守卫的历史依据；应改为“不参与 Rule-R/E；用于审计查询与 TokenId 唯一性守卫”。
  3. **“保留任何 token 即不终止”应限定为“Counter 上的 token”**：`termination_holds` 只检查 Counter；Archive 中的 token 不阻止终止。涉及 TC13、§7 表与 `wait.rs` 注释。
  4. **D3 需补自计数说明**：若 consume gate 的 `count T` 指向目标自身类型，准入时目标仍在 Counter 内、计入阈值；消费后计数减一。当前表述“消费会降低计数”未说明准入时点。
  5. **P7 的 Archival Policy 易被读作内置机制**：`docs/manual.md` §3.2/§6 明确引擎不内置 AP；P7 应表述为“用持 `consumes` gate 的参与者表达，引擎不内置 AP”。
- **覆盖缺口（非不一致）**：`pact-wasm/tests/parity.rs` 已覆盖 `consumes ... when count extra >= 1` 的正反例；尚无“多角色 ack 汇聚后消费共享 token，再清理 ack”的端到端用例。
- **变更**：本轮仅新增审查记录，未改代码、未改其他文档、未提交。

## 2026-10-24（第 50 轮，母本补写消费时机说明）

- **授权**：用户于 2026-10-24 显式授权在母本 §1.6/§1.7.3 补写消费时机说明。
- **母本 §1.6**：在 Design Rationale 后新增 “Consumption timing is a domain policy” 段：Rule-E 不咨询下游需求；共享 token 的读者、未来 Rule-P、`via` 见证与 `count` 阈值；范式无 demand registry、引用计数、last-consumer 信号；`depends_on` 仅 provenance；何时消费须由域指定；早消费会搁浅/终止未来扇出，晚消费会累积并阻止 Rule-T；要求终止的域须为每个 token（含见证、ack）指定最终消费者；Read 不是 reservation；Archive 不在 Counter，不能 read/consume/作 `via` 见证。
- **母本 §1.7.3**：在 temporal limitations 后新增 “Consumption barriers” 段：`consumes t when count ack >= n` 可在消费准入处近似“所有下游完成”的屏障；`count` 是类型级，不绑定消费者实例，也不强制其先完成工作；ack 自身需被消费才能终止；Gate 条件提供结构屏障而非完成协议；read 后 consume 是两步，read 不保留 token。
- **性质**：不新增语法、不改变 Rule-P/R/E/T 语义；§1.6 是对既有非保证的显式化，§1.7.3 的 ack 屏障是既有 `count` 准入的组合用法，未扩展 GateExpr。
- **同步**：`docs/token-consumption-timing.md` §2.1 增母本依据行、§2.2 改为“已补写/仍存边界”状态表、§10 引用落点更新；`docs/design.md` §18 与 `docs/architecture.md` §5 的引用行补注母本依据。
- **笔误修正**：两处搁浅风险引用由 `§A.5` 改为 `§A.6`（搁浅风险实为 A.6；A.5 是良构性分析）：§1.6 Stranding consequence 末尾，以及 Part II“Technical Debt as Unconsumed Tokens”段（第 910 行）。
- **代码**：无代码行为变更、无测试变更；上一轮 `cargo test --workspace` 221 项全绿仍适用。
- **状态**：母本与文档改动未提交、未推送。

## 2026-10-24（第 49 轮，共享 Token 消费时机专题）

- **动机**：母本对“被多个下游使用的 token 何时可以消费”只有零散表述（Rule-P 不消费前驱、Rule-R 幂等、Rule-E 无级联、`via` 见证存活、§1.6 非保证、§1.7.3 时序限制、A.6 搁浅风险），实现侧只有 `rules.rs`/`gate.rs`/`wait.rs`/`prepare_consume`/A.3/A.6 的分散行为，缺少集中判定与模式。
- **新增文档** `docs/token-consumption-timing.md`：参考 `docs/expressiveness-boundaries.md` 的结构，登记 TC1–TC13 能力表（多读者、前驱复用、末位消费者、ack 屏障、副本扇出、见证存活、搁浅、租约、幂等、自动清理、引用计数、外部动作、终止压力）、判定准则 D1–D6、模式 P1–P9、反模式 A1–A9、实现映射、评审清单、引用落点与维护规则。与 B8/B17/B21/B22 交叉引用，不复制边界表，不新增 B/X 编号。
- **文档引用（无行为变更）**：`docs/design.md` §18 增索引行并更新维护句；`docs/architecture.md` §5 增质量属性行；`docs/manual.md` §1.5 增共享 token 消费时机段、§4.5 增幂等非互斥说明；`docs/skill-adaptation.md` E6 增 TC3/TC6/TC13 引用；`examples/README.md` §7.3 增专题入口。
- **代码注释（无行为变更）**：`pact-core/src/rules.rs` `rule_e`（TC2/TC7/TC13）、`pact-core/src/gate.rs` `Via`（TC6/TC7）、`pact-core/src/wait.rs` `termination_holds`（TC13）、`pact-analysis/src/lib.rs` A.3/A.6（TC7/TC10/TC11/TC13）、`pact-engine-kernel/src/lib.rs` `prepare_consume`（TC9）。
- **母本**：**未修改**。若后续要在 §1.6/§1.7.3 补写消费策略或“Read 不是 reservation”声明，须用户显式授权后另开一轮。
- **验证**：`cargo fmt --all -- --check` 通过；`cargo check --workspace` 通过；`cargo test --workspace` **221 项全绿**（无新增/删除测试）。
- **状态**：工作树含本轮文档与注释改动，未提交、未推送；等待用户指示。

## 2026-10-24（第 48 轮，实现侧边界一致性审计）

- **结论**：母本边界声明本身**不需要功能改动**；Rule-P 匹配、GateExpr 作用域、RHS/LHS 唯一性、TokenId 生成、Timestamp 使用均与母本一致。审计另发现 kernel 缺少 `TokenId` 冲突守卫，本轮补齐（见下）。
- **注释对齐（无行为变更）**：
  - `pact-core/src/types.rs`：`TokenId` 改为“Counter 内唯一 + daemon 以 ULID 加强为跨实例唯一”；`ThingRef` 增“范式不定义相等、版本、可变性与基数”；`Timestamp`、`produced_at`、`exhausted_at` 增“仅审计、无排序/因果/新鲜度语义”；`CounterState` 注释同步。
  - `pact-core/src/rules.rs`：`rule_p` 注释明确匹配只读 `TypeName` 与不同 `TokenId`，不读 `ThingRef`；前驱选择策略由调用方决定。
  - `pact-engine-kernel/src/lib.rs`：`prepare_seed` / `prepare_produce` 注释明确调用方必须提供未使用的 `TokenId`（HTTP/engine 以 ULID 生成；kernel 守卫兜底，WASM 宿主同样受保护）。
  - `pact-core/src/effect.rs`：注明重复 `TokenId` 会覆盖既有条目，唯一性由调用方保证，kernel 守卫兜底。
- **母本修订（本轮授权）**：附录 A.5 `LHS_DUPLICATE` remediation 改为“同类型多前驱应建模为不同 `TypeName`（每角色一个）；`count T >= n` 只用于 read/consume 准入，不能表达生产 arity（§1.4.4）”，消除与 §1.4.4 的冲突。
- **TokenId 重复守卫（本轮实现）**：kernel `prepare_seed` / `prepare_produce` 在规则校验通过后检查 Counter 与 Archive，重复则返回 `RuleError::DuplicateTokenId`；HTTP 映射为新错误码，WASM 宿主同步。`Session` 维护 `used_ids: BTreeSet<TokenId>`（恢复时由 Counter + Archive 重建，不改变快照 schema），守卫为 O(log n)；新增 kernel 单测与 HTTP dto 单测。
- **测试适配**：顺序 id 生成器在重启后会从 0 重来，两处测试改用非零起点（`engine_integration.rs` 索引重建重启用 100，`parity.rs` 原生接力打开非空日志用 1000），避免与历史 id 撞号；生产路径使用 ULID，不受影响。
- **验证**：`cargo test --workspace` 221 项全绿；`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --all -- --check` 通过。

## 2026-10-24（第 47 轮，论文边界声明与表达力边界集中化）

- **授权**：用户显式批准对母本做就地一致性修订（不改文法、SOS 规则、定理），范围＝评审最小集 4 处 + `Timestamp` 无语义 1 处 + OR 分支句 1 处。
- **母本修订（6 处，`paper/pactv1_rev3.md`）**：
  1. §1.3 增 `TokenId` 实例身份 vs `ThingRef` referent 身份声明：范式不定义 referent 的相等、版本、可变性与基数；同一 referent 可对应多个 token，反之亦然；对应关系属域/实现。
  2. §1.3 增 `Timestamp` 语义声明：仅记录创建/耗尽时刻供审计，无排序、因果、新鲜度语义；无规则或 gate 读取，不保证单调。
  3. §1.4 Rule-P Note 增：匹配只看 `TypeName` 与不同 `TokenId`，不读 `ThingRef` 及其他字段；前驱选择策略不由范式约束；`depends_on` 是 provenance，不是准入条件。
  4. §1.4.4 Rationale 澄清 `count T >= n` 只作用于 read/consume；生产 arity 由前置图固定；PrecondRule 不能要求同类型多实例。
  5. §1.4.4 Self-loop note 改为 token 实例链表述；对象同一性由域解释 `ThingRef`/`depends_on`。
  6. §1.4.4 OR 分支句改为三类编码（分支各自成链并复制下游 / 统一前驱类型 / 参与者内化），明确 `GateExpr` 的 `or` 仅限 read/consume 准入，且任何编码都不能在保留分支身份的同时汇聚到同一后继类型。
- **新增集中文档** `docs/expressiveness-boundaries.md`：B1–B25 边界编号表、四类判定（可编码 / 仅可近似 / 范式外 / 工具层）、编码模式（OR 分支、同类型多实例、referent 敏感条件、时序与存活、资源与循环、外部编排）、X1–X6 不可编码清单与扩展候选、引用位置与维护规则。
- **引用落点**：`docs/design.md` §18；`docs/architecture.md` §5；`docs/manual.md` §1.5；`examples/README.md` §7.3；`docs/skill-adaptation.md` §7；`pact-core` 的 `DuplicatePrecondLhs` / `ProduceWithCondition`、`pact-analysis::stranding_risk`、`pact-cpn::CpnError` 注释。
- **OR 表述修复**：母本 §1.4.4 原句 “converge at a common successor” 在 RHS Uniqueness 下不可实现，已按第 6 项修订；集中文档 §3.1 与 B4 同步，真正的 OR 汇聚仍列为扩展候选 X1。
- **未变**：文法、SOS 规则、良构约束、事件 schema、代码行为、测试断言；本轮仅文档与注释。
- **验证**：引用路径与 B 编号已逐处核对；无 Rust 行为改动。

## 2026-10-23（第 46 轮，DSH 插件拆分为独立仓库）

- **决策**：按用户裁决采用路径 1，将 DSH 插件从本仓库 `plugins/dsh/` 迁出为独立公开仓库 `onlookor/pact-dsh`，包名由 `@pact/dsh-plugin` 改为未占用的 `dsh-plugin-pact`，版本 `0.1.0`。
- **理由**：插件分发需要包根即仓库根（`dsh plugin add github:...` 与 npm 发布均如此）；GitHub `dsh-plugin` topic 是 repo 级，挂在 Rust 主仓会让整个引擎出现在插件目录中；独立 repo 也使 issue、版本与 DSH 兼容矩阵可以独立演进。
- **新仓库内容**：`plugins/dsh` 的 11 个运行时文件与 7 个测试文件原样迁出，运行时行为未改；`package.json` 补齐 `repository/homepage/bugs/keywords/engines.dsh`，移除 `private`，许可统一为 MIT（原为 Apache-2.0，与仓库 LICENSE 冲突）；新增 `LICENSE`、`.gitignore`、Node 20/22 测试工作流与 `npm run check|test` 脚本；README 重写安装段（npm / `github:onlookor/pact-dsh` / `link:`），并注明来源。
- **本仓库处置**：删除 `plugins/dsh/`；`crates/pact-wasm/js/open-host.js` 与 WASM 产物仍留在本仓库（引擎侧资产），插件通过配置中的文件路径动态引用，不形成包依赖。
- **文档同步**：`docs/requirements.md` FR25、`docs/design.md` D37/§11/§17、`docs/architecture.md` §1/§7、`docs/manual.md` §11/§13、`docs/wasm-local-spike-feasibility.md` 的路径与包名更新；历史变更记录保留“当时位于本仓库”的史实。
- **验证**：新仓库 `npm run check` 通过、`npm test` 47 项全绿、`npm pack --dry-run` 仅含 16 个预期文件（14.4 kB）。本仓库无 Rust 代码变更。

## 2026-10-23（第 45 轮，论文母本纳入仓库管理）

- **决策**：论文母本进入公开仓库，位置由 `docs/` 迁移到顶层 `paper/pactv1_rev3.md`（用户裁决）。理由：论文是规范母本，与需求/设计/手册等实现文档不属同一权威层级；顶层目录使“母本只读、版本化”的纪律可独立表达。
- **版本策略**：保留 rev 文件名，rev3 冻结；后续修订新建 rev4，不就地改写（就地一致性修订仍须用户显式授权）。历史版本以 git 记录为准。
- **清理**：`docs/pactv1_rev1.md`、`docs/pactv1_rev2.md` 已移出项目归档至 `~/pact-paper-archive/`（二者此前被 `.gitignore` 排除、从未进入 git 历史；rev3 为 rev2 的严格超集，rev2 为 rev1 的超集，无独立规范价值）。
- **忽略规则**：`.gitignore` 的 `/docs/pact*` 保留，用于阻止在 `docs/` 下再出现手稿副本；`paper/` 不设忽略规则，正式入库。
- **引用同步**：`docs/requirements.md`、`docs/design.md`（含 D15 历史行的 rev1 措辞）、`docs/manual.md`（引用表与附录链接）、`docs/skill-adaptation.md`、`examples/README.md`、`examples/README.zh-CN.md` 的路径与“论文不入公开仓库”表述已更新为 `paper/pactv1_rev3.md`；历史变更记录中的旧路径作为史实保留。
- **纪律登记**：`AGENTS.md` 新增论文母本条款（唯一规范来源、只保留当前版本、改动须用户显式确认）。
- **文本许可**：新增 `paper/LICENSE`，论文以 **CC BY 4.0** 发布（要求署名、附许可链接、标明改动）；仓库软件仍为 MIT。顶层 `README.md` / `README.zh-CN.md` 的许可段已区分“代码”与“论文手稿”。未修改论文正文。
- **未变**：论文正文、文法、语义、代码与测试均未改动。

## 2026-10-23（第 44 轮，D37 WASM 后端 live 接线）

- **新增宿主组装入口** `crates/pact-wasm/js/open-host.js`（CommonJS）：`openPactHost({module, storeRoot, domain, instance, declaration, ts})` 动态 `require` wasm-bindgen 产物（out-dir 或 `pact_wasm.js`）、构造 `FileStore`、调用 `PactHost.open`；缺省值可由 `PACT_WASM_*` 环境变量提供，非法参数抛 `TypeError`，由插件侧统一包装为 `BackendUnavailable`。
- **插件接线**：新增配置项 `wasmOptions`（纯对象，缺省 `{}`，加载期校验）；根 `loop.js` 将其与 `{domain, instance}` 合并后传给 `createWasmBackend`，实例键仍不可被覆盖。
- **契约对齐（修正）**：`WasmPactBackend.enabled` 原样返回宿主的 Rule-W 可生产选项数组，与插件的布尔契约不符（会永远 fail-closed）；现收窄为“非空即 true”。`close` 增加幂等与已关闭守卫，关闭后调用返回 `BackendUnavailable`。
- **CI 覆盖**：`crates/pact-wasm/js/ci-smoke.js` 增加 `openPactHost` 持久化往返（新建 → seed → 关闭 → 以非法声明重开恢复，验证以日志为准），随 wasm job 执行。
- **验证**：`wasm-bindgen 0.2.127`（用户批准，预编译二进制安装到 `~/.local/bin`，不改 `/usr`）生成 Node 产物；在 `pact-headless` profile 中以 `backend=wasm` 复跑三条路径——允许（bash 实际执行，`isError:false`）、`denyTools` 拒绝、空 Counter 实例的 `agent/pre-step` 拒绝（仅 `turn/start → turn/end`）；拒绝实例的事件日志由插件运行时创建，证明真实经过 WASM 内核。
- **测试**：新增 `test/wasm-backend.test.js`（5 项）与 `config.test.js` 的 `wasmOptions` 断言（2 项）；Node 测试 **47 项**通过，`node --check` 全部通过。
- **未变**：WASM 语义、存储格式、事件 schema、`plugins/dsh` 零 npm 依赖与零 Rust 改动。
- **文档同步**：requirements FR25/变更、design §11 阶段表/§17.2/§17.3/§17.5/§17.6、architecture §7、manual §13、`wasm-local-spike-feasibility.md` §6.1、`plugins/dsh/README.md`。

## 2026-10-23（第 43 轮，D37 真实模型门控复跑）

- **真实模型复跑**：用户提供 `DEEPSEEK_API_KEY`（仅写入 0600 临时文件并经环境变量加载，未落入仓库/日志，验证后删除），在 `pact-headless` profile 中以 `deepseek-official/deepseek-flash` 驱动真实 agent turn。
- **三条路径结果**：
  1. 允许：bash 执行成功，工具结果 `PACT_TOOL_OK`（`isError:false`），模型最终回复 `PACT_TOOL_OK`；
  2. `denyTools:["bash"]`：工具结果 `Error: pact: tool denied by policy`（`isError:true`），模型据此收尾；
  3. `fakeEnabled:false`：会话事件仅 `turn/start → turn/end`，无 `request/header`、无 step、无工具调用，模型请求未发起。
- **结论**：真实模型供应商下三条门控路径与本地 mock 结果一致，验证 `pact-*` preset → `PactAgentLoop` → agent scope 钩子 → DSH turn 的完整链路。
- **未变**：插件代码逻辑本轮零改动（仅此前 `fakeEnabled` 配置项）；41 项测试不变；WASM 后端 live 仍未验证。
- **安全**：凭据未写入任何仓库文件或持久日志；临时 `DEEPSEEK_API_KEY` 文件已删除。
- **文档同步**：requirements FR25/变更、design §17.6、architecture §7、manual §11/§13、README。

## 2026-10-22（第 42 轮，D37 真实会话门控验证：`fakeEnabled` 与三条路径）

- **新增配置项 `fakeEnabled`（默认 `true`）**：`normalizeConfig` 校验布尔值并透传到根 `loop.js` 的 `FakePactBackend({enabled})`；`false` 使所有参与者对 `enabled()` 返回未启用，用于在无 PACT 引擎时预演 `agent/pre-step` 拒绝。仅对 `backend=fake` 生效。
- **真实会话验证方法**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 的 `pact-headless` profile 中注册 `pact-demo` preset、本地确定性 mock LLM 与临时驱动插件；通过 `ctx.agents.create({meta:{agentPreset:'pact-demo'}, setup: mount preset})` 创建 agent 并驱动一个 turn。
- **三条路径结果**：
  1. `govern` + 允许：`bash` 执行成功，工具结果 `PACT_TOOL_OK`，`turn/end` 正常；
  2. `govern` + `denyTools:["bash"]`：`tools/pre-execute` 返回 `{kind:'deny'}`，工具结果 `Error: pact: tool denied by policy`，turn 继续到模型收尾；
  3. `govern` + `fakeEnabled:false`：`agent/pre-step` 返回 `{kind:'reject'}`，`turn/start` 后直接 `turn/end`，未发起模型请求。
- **边界说明**：以上使用本地 mock 模型适配器（无外部凭据），但走真实 `AgentLoop`、`pact-*` 路由、agent scope 钩子与 `ctx.agents.create` 路径；真实模型供应商凭据复跑仍待用户提供。
- **测试**：`config.test.js` 增加默认值、非法类型与显式 `false` 断言；测试总数仍 **41 项**（未新增用例文件）。
- **文档同步**：requirements FR25/变更、design §17.3/§17.5/§17.6、architecture §7、manual §11/§13、`plugins/dsh/README.md`。

## 2026-10-22（第 41 轮，D37 真实 DSH 装配：patch 语义修正 + peer 解析）

- **背景**：安装 `pnpm@11.28.5`（用户前缀）与 `@deepseek-ai/dsh@0.2.1-rc.2`（npx），创建 web profile 并安装 `plugins/dsh/` 进行真实装配。
- **patch 语义修正（D37）**：DSH 的 patch 只按 `id` 覆盖字段或 `disabled`，`name` 仅作一致性校验；原先“把 `agent-loop` 行 `name` 改为 `@pact/dsh-plugin/loop`”会被加载器以 `patch: name mismatch` 跳过。改为：
  - `- id: agent-loop` + `disabled: true`；
  - `- insert:` 同时插入 `pact-agent-loop`（`@pact/dsh-plugin/loop`，同一 `agentLoop` 服务）与 `dsh-pact-observe`。
- **包清单**：`package.json` 增加 `main: "./index.js"` 与 `peerDependencies.@deepseek-ai/dsh-agent-loop ^0.2.1-rc.2`；插件仍无运行时依赖，DSH 运行时按 peer 解析（`link:` 安装时 pnpm 会提示，但实测解析成功）。
- **验证**：`--dump-config` 显示原行禁用与两行插入；启动 web profile 后探针确认 `ctx.agentLoop instanceof PactAgentLoop`、`PactAgentLoop.prototype instanceof AgentLoop`、`Config` 继承同一。
- **未变**：`src/` 路由、钩子、配置、后端、审计与失败策略零改动；41 项 Node 测试不变。
- **仍未验证**：真实模型会话中 `pact-*` agent 的门控触发（需模型凭据与 PACT 后端）；WASM 后端 live。
- **文档同步**：requirements FR25/变更、design §17.2/§17.6/D37、architecture §7、manual §11/§13、`plugins/dsh/README.md`。

## 2026-10-22（第 40 轮，D37：DSH Agent Loop 集成插件）

- **新增 D37**：`plugins/dsh/`（`@pact/dsh-plugin`，ESM、零 npm 依赖）作为仓库内独立 JS 包；不改任何 Rust crate、不进 Cargo workspace、不进 `dist/`。
- **工厂接管**：`cordis.patch.yml` 覆盖 `agent-loop` 行为 `@pact/dsh-plugin/loop`；`PactAgentLoop extends AgentLoop` 仅覆写 `createAgent`/`resume`，`agentPreset` 前缀命中 `pact-` 且 `mode=govern` 时才组合 `setup` 安装钩子，其余原样 `super`。`AgentLoop.Config` 原样复用（不扩 schema）。
- **钩子映射**：`agent/pre-step` → `{kind:'reject'}`；`tools/pre-execute` → `{kind:'deny', reason}`（含 `denyTools` 短路）；`tools/post-execute` 先 `next()` 再审计；`agent/turn-stopping` 仅在 `terminated && stopOnTermination` 时 `steer`；`session/event` 由独立观察行镜像。
- **配置**：`PACT_DSH_CONFIG`（文件或内联 JSON）→ `normalizeConfig`；缺省 observe/fake；`mode=govern` 强制 `failClosed=true`，`backend=wasm` 必须提供 `wasmModule`。错误在加载期 fail-fast。
- **后端**：`PactBackend`（`enabled/gates/status/seed/produce/consume/close`）+ `FakePactBackend`（测试用，不解释门表达式）+ `createWasmBackend`（惰性 `import(module)`，要求 `openPactHost(options)`，缺失时 `BackendUnavailable`）。
- **审计最小化**：只保留 `type/seq/工具名/call id`；`FileSink` 目录 `0700`、文件 `0600`；实例键由 session id 消毒为 `[A-Za-z0-9_-]{1,128}`。
- **实现回填（与 spec 的偏差/补充）**：
  - **补充 1（spec §5.6）**：新增根入口 `plugins/dsh/loop.js`（顶层 `import '@deepseek-ai/dsh-agent-loop'`），使 `src/loop.js` 保持纯工厂、可在无 DSH 依赖的 Node 环境测试。
  - **补充 2（spec §5.2）**：`createWasmBackend` 支持注入 `loadHost`，便于后续接入真实 `pact-wasm` 包；`module` 必须导出 `openPactHost`（非既有 `pact-host.js` 的构造形式），属阶段 12 接线项。
  - **补充 3（spec §5.5）**：`denyTools` 命中时直接 `{kind:'deny'}`，不调用后端，避免策略拒绝依赖后端可用性。
  - **偏差 1（spec §5.6）**：`resume` 在 `super.resume` 返回后按 `session.header.meta.agentPreset` 安装钩子，恢复后的首步不受门控。
- **测试**：新增 6 个 Node 测试文件（配置/命名/假后端/观察者/治理/路由），`node --test` **41 项**通过；`node --check` 全部通过。无 Rust 改动，未跑 `cargo test`。
- **未验证**：真实 DSH 装配（未安装 dsh/pnpm）、WASM 后端 live（无 wasm-bindgen 产物）、`AgentLoop`/钩子签名兼容性；均受环境限制，需另行审批。
- **安全审视（2026-10-22，零发现/已加固）**：无网络监听、无子进程、无 `eval`/`new Function`、无 `console` 日志；审计只落允许字段（参数/结果不入库，测试覆盖）；session→实例键消毒 + `domain`/`participant` 白名单 + `participants` 使用 null 原型（防原型污染）；审计目录 `0700`/文件 `0600`；sink 失败与后端异常不逃逸到 harness；`govern` fail-closed、`observe` fail-open 且留痕；`next()` 恰好一次；`turn-stopping` 读失败不阻断收尾；动态导入仅来自操作者配置的 `wasmModule`（受信配置边界）；无新增依赖、无 Rust/unsafe 改动。发现的 1 处加固：`normalizedParticipants` 改为 `Object.create(null)` 并新增测试。
- **文档同步**：requirements FR25、design §17 与阶段 11 行、architecture §7、manual DSH 一节、`wasm-local-spike-feasibility.md` §6.1、`plugins/dsh/README.md`。

## 2026-10-22（第 39 轮，NFR6 v2：D34 TLS/签名、D35 MCP HTTP、D36 OIDC）

- **新增 D34 TLS 与 Ed25519 请求签名身份**：`[tls] certificate/private_key/client_ca`，仅 TCP 生效，rustls 终止于 pactd，`client_ca` 即 mTLS；`[auth.keys]` 非空启用签名，四头 + `PACT-SIGNATURE-V1` 规范串、±300s、nonce TTL 300s/8192、8 MiB、401/403/413/429；签名与 Bearer 并存且签名优先；可信头 `x-pact-verified-principal` 与内部能力头 `x-pact-internal-auth`。ACL 身份来源扩展为「Bearer 或签名」。
- **新增 D35 MCP Streamable HTTP**：工具层下沉 `pact-http::mcp`（17 工具），daemon `[mcp].enabled` 挂 `/mcp`（rmcp `StreamableHttpService` + `LocalSessionManager` + legacy session mode）；本地 tower 调用自身 REST；`pact-mcp` 保留 stdio 薄封装；`allowed_hosts = 回环 + public_url authority`；信任模式仅无 OIDC/无令牌时启用并告警。
- **新增 D36 MCP OIDC 资源服务器**：`[mcp.oidc]` 全套配置；发现/JWKS 缓存 300s、最多 64 键、未知 kid 限频 30s；alg 白名单、iss/aud/exp/nbf、60s leeway、claim→principal 白名单；`GET /.well-known/oauth-protected-resource` 与 401 `WWW-Authenticate`；IdP 不可达 503；仅 `/mcp`、不缓存、不转发 REST。
- **配置文档**：§9.4 扩为完整 `pact.toml` 示例（`[tls]`、`[auth.keys]`、`[auth.tokens]`、`[auth.acl]`、`[mcp]`、`[mcp.oidc]`），并列出 `Config::validate()` 的拒绝条件。
- **表述澄清**：启用 `[auth.keys]` 后 REST 非豁免请求必须签名，Bearer 不能替代签名；两者同时出现时以签名身份为准（§9.4/§9.6/§9.10 同步）。
- **章节重排**：新增 §9.5 TLS、§9.6 签名身份、§9.7 MCP HTTP、§9.8 OIDC；原 §9.5 运维接口、§9.6 域级 ACL 顺延为 §9.9、§9.10，相关交叉引用同步（D25、§11 阶段 8）。§9.2 端点表补 `/mcp` 与元数据；§12 改为 v2 已实现；§14 安全边界删除「v1 无 TLS」。
- **实现回填（与 spec 的偏差/补充）**：
  - **偏差 1（spec §5.8）**：`pact_http::mcp::http_service(client, allowed_hosts)` 比 spec 多一个 `allowed_hosts` 参数——Host 头保护需要回环 + `public_url` authority，不能在函数内部从 `Client` 获取。
  - **偏差 2（spec §5.8）**：rmcp `StreamableHttpServerConfig` 以 **builder**（`StreamableHttpServerConfig::default().with_legacy_session_mode(true).with_allowed_hosts(...)`）构造，spec 的字段式构造不存在。
  - **接口补充（spec §5.6）**：`ServerState::with_identity(engine, tokens, keys, acl, oidc, public_url)` 比 spec 多 `public_url`，用于 `/mcp` 的 `WWW-Authenticate` 与元数据 `resource`。
  - **接口补充（spec §5.9）**：`router_with_mcp(state, mcp_enabled)` 只接收状态与开关，内部调用 `router(state)` 建基路由并从状态取 `public_url` 组装 `allowed_hosts`（回环 + 该 authority）；`/mcp` 鉴权中间件为 `require_mcp_auth`，与 `enforce_identity` 分离。
  - **接口注记（spec §5.7）**：`ProtectedResourceMetadata` 未包含可选的 `scopes_supported`；实现只返回 `resource` / `authorization_servers` / `bearer_methods_supported`（RFC 9728 允许省略）。
  - **已知限制（spec §5.11）**：`pact-runtime` 的 stdio MCP 子进程当前只透传 `--https/--tls-ca/--bearer`，未透传 `--sign-key/--sign-as`；`AgentSpec/BootstrapSpec` 也未提供每 agent 签名身份。连接签名 daemon 的 runtime MCP 通道需改用 REST 或 TLS+Bearer。
- **测试**：新增 `crates/pact-http/tests/nfr6_v2.rs`（TLS/mTLS、签名/重放/篡改、MCP Bearer、OIDC 正负、元数据、IdP 不可达）；`cargo test --workspace` 219 项通过。
- **安全审视加固（18 项逐条核验）**：
  - 签名中间件顺序调整为「公钥查表 → Ed25519 验签 → nonce 去重」：未通过签名的请求不能写入防重放缓存，消除未认证请求填满 8192 条缓存导致的 429 拒绝服务放大。
  - 进程内能力令牌优先从 `/dev/urandom` 读取 32 字节；不可用时退化为两枚 ULID（160 位随机熵），与文档「32 字节随机」一致。
  - OIDC 的 `jwks_uri`（含发现文档返回值）强制 `https://`，仅 `allow_insecure_http = true` 时允许 `http://`，防止 IdP 响应导致的明文降级/非 HTTP(S) 目标。
  - 其余项（无 `unsafe`、私钥只读、常量时间比较、可信头剥离、豁免精确匹配、CLI 无密钥回显、依赖均来自 crates.io）核验通过。

## 2026-10-22（第 38 轮，CI 修复：wasm job 工作流解析失败）

- **现象**：run 39616524738（39c210d）与 39617616804（6dba5e3）均在启动前失败，未产生任何 job。
- **根因**：`.github/workflows/ci.yml` 在 **job 级 `env`** 中使用 `${{ runner.temp }}`；GitHub Actions 仅在 step 级允许 `runner` 上下文，解析期报 `Unrecognized named-value: 'runner'`。
- **修复**：移除 job 级 `PACT_WASM_PKG`，新增准备步骤将 `PACT_WASM_PKG=${RUNNER_TEMP}/pact-wasm-pkg` 写入 `$GITHUB_ENV`，后续 bindgen/冒烟/Node 测试继承该变量。
- **验证**：run 39763458339（22a5517）成功；`wasm32 构建 / ABI 与体积门禁 / Node 冒烟` 与 `build / test / fmt / clippy` 两个 job 均为 success。
- **不变**：工作流职责、门禁阈值与 job 划分不变。

## 2026-10-21（第 37 轮，D33：去除 WASM “实验性”标记，转为受支持本地分发形态）

- **触发**：FR21 稳定化验收 (1) Q7 CI、(2) 金标扩面、(3) `pact-engine-kernel` 抽取、(4) 宿主边界安全审视全部完成；用户裁决去除“实验性”标记。
- **决策 D33**：`pact-wasm` 由“实验性可选分发形态”转为**受支持本地分发形态**。定位与边界不变：权威仍是原生 daemon；仅 Node/桌面本地单用户嵌入；不提供 MCP/HTTP、ACL/审计、浏览器 OPFS/SharedWorker、DSH `govern` 与 NFR6 v2；不进入 `dist/` 产物集，不替代 `pactd`。
- **文档同步**：
  - `design.md` 新增 D33；crate 清单与 §16 标题/16.1/16.4 去除“实验性”，并登记稳定化验收完成与维护约束（wasm CI 持续执行、语义改动须通过原生/WASM 金标对账）。
  - `requirements.md` FR21 改为“已稳定（D33）”，保留四项验收与能力边界；§6 范围外表述同步。
  - `architecture.md` §1 改为“受支持（D31/D33）”。
  - `wasm-local-spike-feasibility.md` 状态、§10、§11.2–11.4 同步。
  - `crates/pact-wasm/Cargo.toml` description 去除“实验性”。
- **不变**：ABI `pact-wasm/0.2`、事件 schema、磁盘布局、依赖方向、错误码、CI 门禁与 `dist/` 产物。
- **验证**：仅文档/描述变更，无代码行为变化；`cargo check -p pact-wasm` 通过。

## 2026-10-21（第 36 轮，D32：抽取 `pact-engine-kernel`，原生与 WASM 共用实例会话语义）

- **新增纯内核 crate `pact-engine-kernel`**（无 IO、无 tokio；依赖 `pact-core`/`pact-parser`/`pact-analysis`/`pact-store-model`/`pact-views`）：
  - `Session` 统一持有 `declaration`/`domain`/`state`/`last_seq`/`next_seq`/`dl_seq`/`terminated`/`consumed_requests`；
  - 提供 `fresh`、`recover`（快照锚点 + 日志重放 + 幂等表 + 终止态）、`prepare_seed`/`prepare_produce`/`prepare_consume`/`read`、`mark_prepared`/`abort_prepared`（WASM 两阶段提交）、`commit`/`record_terminated`、`counter`/`archive`/`gates`/`enabled`/`status`/`snapshot`/`summary` 与 `analysis_report`；
  - `KernelError` 聚合 Parse/Rule/Model/Recovery/IdempotencyConflict，由原生与 WASM 各自映射到既有错误码（对外错误码不变）。
- **原生迁移**：`pact-engine` 的 `InstanceSession` 改为 `{ core: Session, events_since_snapshot, notify }`，经 `Deref`/`DerefMut` 复用内核方法；`recover_instance`、`instantiate`/`load_domain` 重置、`seed`/`produce`/`consume`/`read`、查询视图与 `analyze` 全部委托内核；`post_commit` 接收 `terminated_seq` 并在追加 `Terminated` 后调用 `record_terminated`。
- **WASM 迁移**：`pact-wasm` 的 `InstanceSession` 改为 `{ core: Session, events }`，删除约 500 行重复状态逻辑；`prepare_*`/`commit`/`abort`/`restore`/视图/分析全部委托内核。
- **行为契约不变**：DomainLoaded seq 从 1 起；失败命令仍推进时钟；Rule-T 终止 `seq = last_seq + 1` 且只报告一次；压缩水位 `base_seq` 恢复；Rule-E 幂等表跨快照恢复；`ConsumePreparation::Prepared` 以 `Box` 承载完整事件（控制枚举尺寸）。
- **前置进度**：FR21 稳定化前置 (3) 标记完成；仅余 (4) 推送/发布前宿主边界安全审视。
- **验证**：`cargo test --workspace` 184 项全绿（含 kernel 3 项单测、parity 7 项）；`cargo fmt --all --check` 通过；原生 `clippy --workspace --all-targets -D warnings` 与 wasm32 `clippy -p pact-wasm -D warnings` 均零告警；wasm-release 构建 + bindgen + `ci-smoke`（raw 455,929 B / gzip 142,236 B）+ Node 7 项测试通过。
- **未变**：事件 schema、磁盘布局、ABI `pact-wasm/0.2`、原生接口与错误码、依赖方向、`dist/` 产物。

## 2026-10-21（第 35 轮，金标扩面：多实例 / 组合域 / 条件分支 / 压缩恢复）

- **测试覆盖扩展**（`crates/pact-wasm/tests/parity.rs`，新增 4 项，共 7 项 parity）：
  1. `multi_instance_isolation_matches_native`：同一域声明并发两实例，各自独立 `DomainLoaded`/seq/idempotency；逐实例事件日志与原生逐行相等，Counter/Archive 隔离，`list_domains` 实例清单一致。
  2. `composed_domain_matches_native`：两片段经 §1.8 组合后 `DomainSummary` 完全相等；种子/生产事件日志逐行相等；四个参与者的 `my_gates` 逐项相等。
  3. `conditional_gate_branches_match_native`：`count T >= 2`、`count T >= 1`、`not exists T` 三种 GateExpr 分支的放行与 `GateConditionFailed` 拒绝逐项对账，含失败命令后的时钟推进差异。
  4. `compacted_log_recovery_matches_native`：原生 `compact(keep_last=0)` 后「快照 + 空日志」由 WASM `restore` 恢复，视图相等；WASM 续写 seq 接续快照水位（尾部仅 1 行），原生 daemon 可再打开该尾部日志并得到一致视图。
- **前置进度**：FR21 稳定化前置 (2) 金标扩面标记完成；(3) `pact-engine-kernel` 抽取与 (4) 安全审视仍待办。
- **验证**：`cargo test -p pact-wasm` 15 项全绿（8 单测 + 7 parity）；`cargo clippy -p pact-wasm --all-targets -D warnings` 零告警；`cargo test --workspace --locked` 全绿。
- **未变**：实现代码、事件/磁盘格式、ABI、CI 门禁；本轮仅扩展测试与文档。

## 2026-10-21（第 34 轮，Q7：WASM 独立 CI job）

- **新增 CI job `wasm`**（`.github/workflows/ci.yml`）：安装 stable + `wasm32-unknown-unknown` → wasm 目标 `clippy -D warnings`（仅 lib，避免 `--all-targets` 拉入 `pact-engine`/`tokio` 等原生 dev-deps）→ `cargo build -p pact-wasm --profile wasm-release` → `wasm-bindgen --target nodejs` → ABI/语义/体积门禁 → Node 持久化测试。
- **wasm-bindgen CLI 版本锁定**：`0.2.127`（须与 crate 版本一致），经 `actions/cache` 缓存 `~/.cargo/bin/wasm-bindgen`；未命中时才 `cargo install wasm-bindgen-cli --version … --locked`。
- **新增门禁脚本** `crates/pact-wasm/js/ci-smoke.js`：断言 `abi_version() === "pact-wasm/0.2"`；`validate`/`analyze` 最小语义路径（类型 2、gate 子句 4、seed_authority 1、A.5 良构、源类型可达）；体积上限 raw ≤ 600 KiB、gzip ≤ 200 KiB（基线 450,190 B / gzip ~140 KiB，留约 30% 余量）。
- **设计 §10 测试策略**新增「WASM 门禁」行。需求 FR21 的稳定化前置 (1) 标记为已实现，其余 (2)–(4) 不变。
- **本地实测**：wasm32 clippy 清零；wasm-release 构建与 bindgen 通过；`ci-smoke` 通过（raw 450,190 B / gzip 140,289 B）；Node 7 项持久化测试全绿。
- **未变**：crate 代码、事件/磁盘格式、原生 CI `check` job；本轮只加 CI 与门禁脚本。

## 2026-10-21（第 33 轮，P6 裁决 A：WASM 本地分发形态条件立项（D31））

- **新增决策 D31**：WASM 本地运行登记为**实验性可选分发形态**（Node/桌面宿主），权威仍是原生引擎；`pactd`、NFR4 单静态二进制与 FR18 生产分发承诺不变。
- **新增 §16「WASM 本地分发形态（实验性）」**：16.1 定位与权威边界（本地单用户、无 ACL/审计、不提供 MCP/HTTP）；16.2 组件与宿主职责（`pact-wasm` 内核 / JS 宿主适配层 / 原生 store+engine 三方边界）；16.3 ABI `pact-wasm/0.x` + JSON 字符串与 `prepare → append+fsync → commit` 唯一写路径（abort 回收 seq、`Terminated` 派生追加、ID/ts 宿主注入）；16.4 稳定化前置（Q7 CI → 金标扩面 → `pact-engine-kernel` 抽取 → 安全审视）与不纳入项（浏览器 OPFS/SharedWorker、多用户、压缩/索引、DSH `govern`、NFR6 v2）。
- **crate 清单注释**：`pact-wasm` 由「spike S1」改为「WASM 本地分发形态（实验性，D30/D31）」。
- **未变**：事件 schema、磁盘布局、原生 API、依赖方向、`dist/` 产物；本轮仅登记立项、分发形态与 ABI 策略，未改代码。
- **配套**：需求侧新增 FR21（`docs/requirements-change.md` 第 18 轮），架构侧新增组件边界（`docs/architecture-change.md` 第 20 轮），可行性报告状态改为已条件立项。

## 2026-10-21（第 32 轮，P4/S2：Node 持久化宿主）

- **JS 存储适配器**（`crates/pact-wasm/js/pact-store.js`）：与原生同布局（`<root>/<domain>/<instance>.{events.jsonl,snapshot.json,lock}`）；`append` = `open('a') + write + fsync + close`；`writeSnapshot` = tmp + `fsync` + `rename` + 目录 `fsync`；`readEvents` 用字节级 `lastIndexOf('\n')` 截断崩溃残留半行并 `fsync`；域名/实例名以 `[A-Za-z0-9_-]{1,128}` 校验，杜绝路径穿越。
- **单写者**：`.lock` 以 `O_EXCL` 创建并写 pid；冲突时若持有者进程不存在（`kill(pid,0)` 失败）则回收一次，否则抛 `InstanceLocked`；`close` 删除锁。仅进程级近似，不等价于原生 `flock`（已在文档标注）。
- **最小宿主**（`pact-host.js`）：`PactHost.open` 获取锁后按「日志/快照优先」恢复，空日志且无快照才新建并追加 `DomainLoaded`；`_finish` 严格 `prepare → append+fsync → commit`，追加失败 `abort`；`commit` 返回的 `Terminated` 事件随后追加（信息性，失败不回滚状态）。
- **demo**（`js/demo.js`）：生命周期 + 快照 + 重启校验，便于人工观察。
- **测试**：Node `node:test` 7 项（重启一致、append 后未 commit 的崩溃恢复、半行截断、单写者、快照自足、幂等跨重启、Rule-T 终止落盘）；Rust 新增 `parity.rs::native_and_wasm_alternate_on_shared_log`（原生→WASM→原生→WASM 交替打开同一日志，视图一致）。全仓 **177 项测试全绿**，fmt/clippy（native + wasm32）清零。
- **边界**：浏览器 OPFS/SharedWorker 不做（Q6）；WASM CI job 不做（Q7 待决策）；不改需求、不动原生磁盘格式。

## 2026-10-21（第 31 轮，P3/S1：WASM 内存语义与原生对账）

- **实例会话**：`PactHost` 由「声明缓存」升级为「声明注册表 + `(domain, instance)` 会话表 + 待提交事务表」；每会话持有声明快照、`CounterState`、完整事件序列、`last_seq`/`next_seq`、`dl_seq`、`terminated` 与 Rule-E 幂等表。`is_occupied = last_seq > dl_seq` 与原生一致。
- **生命周期对齐原生**：`load_domain` 空实例可重置（seq 续接）、非空实例返回 `DomainBusy`；新增 `load_domain_with(..., force)` 显式替换；`instantiate` 从已登记声明创建额外实例；`load_domain_at` 注入确定性时间戳（`DomainLoaded` 现参与事件序列，P2 的 `last_seq = 0` 由此变为真实值）。
- **两阶段提交**：`prepare_seed/produce/consume` 校验并分配 seq，返回 `Prepared { tx_id, seq, event(JSONL 行), result, replayed }`；`commit(tx_id, terminated_ts)` 应用事件、写入幂等表、按原生 `post_commit` 顺序追加 Rule-T `Terminated`（可选独立时间戳）；`abort` 丢弃事务并回收未被后续分配占用的 seq，保证 JSONL 连续、可被原生 `Store` 的 `seq == last+1` 校验接受。`prepare_consume` 幂等命中返回 `tx_id = None`、`replayed = true`，不产生事件。
- **查询**：`read`（Rule-R）与 `counter`/`archive`/`gates`/`enabled`/`status`，直接复用 `pact-views` 类型与 `pact-core` 规则，字段顺序与原生一致。
- **导出/恢复**：`export_events` 逐行 `to_wire_json`（顶层 `version`）；`export_snapshot` 复用 `Snapshot::new`（自足）；`restore` 复刻 `recover_instance` 的锚点选择（最后一个 `DomainLoaded` 优先，否则快照锚）、`base_seq` 重放、幂等表重建与终止态计算。
- **错误码**：`FacadeError.code` 由自造小写改为与原生接口层同名（`InstanceNotFound`/`DomainBusy`/`InstanceExists`/`DomainNotDeclared`/`IdempotencyConflict`/`SeedAuthorityDenied`/`PredecessorNotOnCounter`/`TokenNotOnCounter`/`InvalidDomainName`/`InvalidInstanceName`/`Syntax`/`WellFormedness`）；`FacadeError.code` 类型改为 `String`。
- **ABI**：`abi_version` 升至 `pact-wasm/0.2`。新增导出 `load_domain_at`/`load_domain_with`/`instantiate`/`prepare_seed`/`prepare_produce`/`prepare_consume`/`commit`/`abort`/`read`/`counter`/`archive`/`gates`/`enabled`/`status`/`export_events`/`export_snapshot`/`restore`。
- **测试**：`crates/pact-wasm/tests/parity.rs` 两项（金标事件/视图/双向互通、错误码对账）——固定 `T0–T8` + `seq-0..2`，**8 条事件与原生 JSONL 逐行相等**（含 `Terminated`），视图与错误码逐项相等，WASM 日志+快照可被原生 daemon 恢复，原生日志可被 WASM 恢复。`pact-wasm` dev-dependencies 新增 `pact-engine`/`pact-store`/`tokio`/`tempfile`（仅测试）。全仓 **176 项测试全绿**（P2 的 171 + 5），fmt/clippy（native + wasm32）清零。
- **体积**：`wasm-release` 产物 bindgen 后 450,190 B（440 KiB）、gzip 140,457 B（137 KiB），较 S0 的 267 KiB 增加 173 KiB，登记为新基线。
- **边界**：仍不落盘（P4 的 JS `PactStore` 负责 append/fsync/崩溃恢复）；不接 CI（Q7 待 P3 退出后再议）；浏览器 OPFS/SharedWorker 不做（Q6）。

## 2026-10-21（第 30 轮，P2/S0：WASM 门面 `pact-wasm`（D30））

- **新增 `pact-wasm`**（`crate-type = ["cdylib", "rlib"]`）：宿主无关的 `PactHost` 承载解析/校验/静态分析/片段组合；`wasm-bindgen` 包装仅编译到 `wasm32`，原生构建不拉入该依赖。
- **ABI（Q2 冻结）**：全部方法使用 JSON 字符串；导出 `new`、`load_domain(name, declaration, instance)`、`validate(declaration)`、`analyze(declaration)`、`compose(name, fragments_json, instance)`、`list_domains`，另有 `abi_version` 探针；错误以 `{"code","message"}` JSON 字符串抛出。
- **构建配置**：根 `Cargo.toml` 新增 `wasm-release` profile（`opt-level="z"`、LTO、`codegen-units=1`、`panic="abort"`、`strip`）；`wasm-bindgen 0.2.127` 仅作为 wasm32 目标依赖。
- **实测**：release `.wasm` 273,394 B（267 KiB），gzip 89,054 B（87 KiB）；Node 22 模块加载 1.71 ms、构造 0.35 ms、首次 `analyze` 3.24 ms。
- **语义对账**：`react_loop.pact` 与 `cross_harness.pact` 的 `analyze` JSON 与原生 `pact --json analyze` 深度相等。
- **边界**：S0 不写事件、不落盘、无 `prepare/commit`（P3/P4）；浏览器绑定、OPFS、wasm-pack 与独立 CI job 均未纳入（Q6/Q7）。
- **验证**：`cargo fmt --all --check`、host 与 wasm 目标 `clippy -D warnings` 均干净；全仓 **171 项测试全绿**（P1 的 166 + `pact-wasm` 5）。

## 2026-10-21（第 29 轮，P1：WASM 纯类型层拆分（D29））

- **新增 `pact-store-model`**：承载 `Event`（五变体）、`WireEvent`（顶层 `version` + `flatten`）、`EVENT_SCHEMA_VERSION`、`apply_event`/`ApplyOutcome`、`to_wire_json`、`Snapshot` 数据结构、`InstanceKey` 与 `ModelError`；依赖仅 `pact-core + serde + serde_json`，无 IO/tokio/unsafe，可编译到 `wasm32-unknown-unknown`。
- **新增 `pact-views`**：承载 `CounterView`、`ArchiveView`、`StatusView`、`GateClause`、`DomainSummary`、`DomainListing`、`CompactionOutcome`、`AnalysisReport`；依赖仅 `pact-core + pact-analysis + serde`，无 tokio，可编译到 wasm。
- **原生 API 保持**：`pact-store/src/event.rs` 与 `key.rs` 删除，`lib.rs` 从模型 crate 原样 re-export（`Event`/`WireEvent`/`InstanceKey`/`Snapshot`/`apply_event`/`EVENT_SCHEMA_VERSION` 等）；`pact-store::snapshot` 只保留原子读写与版本校验。`pact-engine/src/views.rs` 改为 `pub use pact_views::*`，`pact_engine::views::*` 路径不变。
- **错误边界**：`ModelError`（JSON/非法域名/非法实例名/不支持的 schema 版本）为纯模型错误；`StoreError` 保留全部既有变体并新增 `From<ModelError>` 一一映射；`EngineError` 新增 `From<ModelError>`（转 `Store`），调用方匹配路径不变。
- **零格式变化**：`Event`/`Snapshot` 的 serde 属性逐字迁移，JSONL 与线上 JSON 字段、默认值、`skip_serializing_if` 均未改；磁盘布局、索引 schema、压缩水位均未改。
- **测试**：事件/快照模型的原测试随类型迁入 `pact-store-model`；全仓 `cargo test --workspace` **166 项全绿**（与拆分前基线一致）；`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo check --target wasm32-unknown-unknown -p pact-store-model -p pact-views` 通过。
- **未执行**：`pact-wasm` 门面、JS 存储适配、`prepare/commit` 实现仍属 P2/P3；本轮不接线、不改 `docs/pact*` 母本。

## 2026-10-21（第 28 轮，阶段 6：CPN 编码生成 pact-cpn（D28））

- **D28 新增**：`pact-cpn` 纯函数生成器 + CLI `pact cpn`；输出 CPN Tools XML（`generator format='6'`）；单实例作用域命名空间 `i_<instance>__`；颜色 `PACT_TOKEN = TokenId × Participant × ThingRef × PrecondRefs × Timestamp`、`PACT_ARCHIVE = PACT_TOKEN × Participant × Timestamp`、`PACT_COMPL = int`。
- **结构映射**：每类型库所 + Archive；生产迁移对每个前驱一条 test arc（输入绑定 + 同变量输出），输出依赖列表 `[#1 xi]`；读迁移 `x` 自环；消费迁移消耗 `x` 并输出 Archive；种子迁移无输入弧。
- **条件编码**：`GateExpr → NNF → DNF`；每个合取项一条迁移（`or` 多迁移，名称 `_c1/_c2…`）；`count T >= n` n 条 test arc + 两两不同 guard；`via T` test arc + `List.exists (fn z => z = #1 y1) (#4 x)`；`count T <= k` 用互补库所（初值 N、`M(p_T)+M(p_T_not)=N`、生产/种子消耗 1、消费归还 1、测试弧 N-k 借还）；`not via` 拒绝；缺上界/超大 DNF/`count >= n` 超过 1024/非良构域分别报错。
- **目标类型自查**：目标 token 计入目标类型 count，故 `exists T`/`count T >= 1` 在 T 为目标时无需额外弧，`count T >= n` 只需 n−1 个额外见证；`via T` 在目标类型上仍需额外见证。
- **CLI 接入**：`pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`，`--json` 输出 `CpnStats`；错误映射为 `Cpn` 代码（缺上界提示具体类型）；离线路径不触 daemon。
- **设计文档**：决策表新增 D28；§1 crate 清单与依赖图新增 `pact-cpn`（`cpn → core`）；新增 §15 完整编码设计；阶段表阶段 6 标记已完成。顺手修正 D25 行中与 §9.6 不一致的「(3) 未开始」为「已实现（2026-10-20）」。
- **验证**：`pact-cpn` 11 项测试（结构计数、via/test arc、not 互补、目标自查、or 展开、count 上界、not via 错误、实例命名空间、XML 良构、超限字面量拒绝）；CLI 离线 e2e 覆盖成功与缺上界；`examples/domains/*.pact` 生成物全部通过 `xmllint --dtdvalid`（CPN DTD）；`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **166 项全绿**。
- **未执行**：CPN Tools GUI 内实际加载与状态空间实验（沙箱无 GUI）——列为人工验收；`--bound` 正确性由调用方承担，文档已明确。
- **安全审视（零可利用发现）**：生成器零 IO、无 `unsafe`、无新增第三方依赖（仅 pact-core + serde derive）；所有输出文本经 XML 转义，名称经 ML 标识符安全化，实例名进入命名空间前消毒；DNF 设 1024 项上限防指数膨胀；错误路径不回显文件内容。

## 2026-10-20（第 27 轮，阶段 9：分发产物 dist/（D26））

- **交付形态**：新增 `dist/` 六个文件——`Dockerfile`（多阶段 `rust:1.98-alpine` → `alpine:3.22`，非 root uid/gid 10001、`/var/lib/pact` 数据卷、`EXPOSE 8787`、`HEALTHCHECK` 探 `/health`、入口 `pactd --config /etc/pact/pact.toml`）、`Dockerfile.dockerignore`（排除 `.git`/`target`/`docs`/`examples`/本地数据，避免论文母本进入构建上下文）、`pact.container.toml`、`pactd.service`、`pact.toml.example`、`build-static.sh`。
- **systemd unit**：`User/Group=pact`、`StateDirectory=pact`（`/var/lib/pact` 0700）、`RuntimeDirectory=pact`（`/run/pact` 0700）、`Restart=on-failure`、`KillSignal=SIGTERM` + `TimeoutStopSec=20s`（覆盖 10s drain）、`UMask=0077`；加固 `ProtectSystem=strict`/`ProtectHome`/`PrivateDevices`/`ProtectKernel*`/`ProtectControlGroups`/`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`/`RestrictNamespaces`/`LockPersonality`/空 `CapabilityBoundingSet`。
- **静态打包**：`build-static.sh` 缺省目标 `x86_64-unknown-linux-musl`；自动探测 `musl-gcc` 或 musl 系 `cc` 并导出 `CC_<target>`/`CARGO_TARGET_<target>_LINKER`（turso 的 C 依赖需要）；`cargo build --release --locked --bins`；strip 后逐 bin `readelf -d` 无 `NEEDED` 校验；归档含 `bin/`、unit、配置示例、`LICENSE`、`SHA256SUMS`，并产出 tar.gz 与 `.sha256`；缺前置显式失败，不回退 glibc。
- **配置示例**：systemd 侧 `/var/lib/pact` + `/run/pact/pact.sock`；容器侧 `listen_unix=""` + `0.0.0.0:8787`，文档强制 `-p 127.0.0.1:8787:8787`（v1 无 TLS）。
- **形式层与代码零改动**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` / `pact-store` / `pact-engine` / 三接口与 `pact-runtime` 均未修改；`docs/pact*` 母本未动；`dist/**` 按 D26 不接入 CI 语义测试。
- **验证**：`bash -n dist/build-static.sh` 通过；两份 TOML 由 Python `tomllib` 解析通过；用临时目录替换路径后，`pactd` 实际以两份示例配置启动并优雅停机（unix 套接字与 TCP 两种监听均正常）；`systemd-analyze verify`（临时 root，补齐目标 stub）单元语义通过；`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **155 项全绿**（无代码改动，与上一轮基线一致）。**未执行**：musl 静态构建与容器镜像构建——当前沙箱无 musl C 工具链、无 Docker/Podman；脚本对缺失前置显式失败，实际构建需在具备 `musl-gcc` 的主机（或 Docker 构建阶段）执行。
- **全面安全审视（零可利用发现）**：
  - **构建上下文**：`Dockerfile.dockerignore` 排除 `.git`、`docs`（含未公开论文母本 `docs/pact*`）、`examples`、`target` 与本地数据；该机制要求 BuildKit（Docker 23+ 默认），未使用经典构建器，手册与 Dockerfile 均标注。
  - **镜像运行**：非 root uid/gid 10001；数据目录为卷；运行阶段不额外 `apk add`（仅基础镜像 busybox，无额外 CVE 面）；配置为无密钥示例。
  - **网络暴露**：容器内 `0.0.0.0:8787` 仅为端口映射可达，手册强制 `-p 127.0.0.1:8787:8787`；v1 无 TLS 且默认信任声明模式，跨主机使用须外部 TLS + `[auth.tokens]`/`[auth.acl]`（已在 §2.4/§7.6 与示例注释登记）。
  - **systemd 权限**：`StateDirectory`/`RuntimeDirectory` 0700、`UMask=0077`、空 capability 集、`ProtectSystem=strict`/`ProtectHome`/`PrivateDevices`/`ProtectKernel*`；只放开 unix/inet 地址族；`TimeoutStopSec=20s` 覆盖 10s drain。配置文件按 `root:pact 0640` 安装（可含令牌）。
  - **打包脚本**：无 `eval`；strip 后以 `readelf -d` 拒绝任何 `NEEDED` 动态依赖（防误发 glibc 产物）；归档不含密钥或未公开文档；校验和未签名，已在设计 §14 列为非目标。
  - **代码面**：零 Rust 改动、无 `unsafe`、无新增依赖；`docs/pact*` 母本未动。

## 2026-10-20（第 26 轮，D25 子项 3：域级 ACL 与读审计视图实施）

- **四项设计取舍（用户裁决，均选推荐项）**：ACL 粒度 = 实例级条目 `domain/instance` + `domain/*` 通配；ACL 范围 = 生命周期（load / instantiate / unload / compact / `:analyze` / `:compose`）+ 四类动作 + 查询 + 读审计视图，未配置时保持既有行为；读审计 = 独立 `<instance>.read-audit.jsonl`（append + fsync），不进入主事件日志；审计语义 = fail-closed + REST 只读视图。
- **配置（pact-http）**：`Config.read_audit: Option<bool>`（缺省 false）；`AuthConfig.acl: BTreeMap<String, Vec<String>>`；新增 `Config::{acl(), read_audit(), validate()}`。`validate()` 拒绝「ACL 非空 + 令牌表为空」及非法条目（只接受 `domain/instance` / `domain/*`），daemon 启动前调用。匹配函数 `acl_allows` / `acl_allows_domain` 仅做字符串比较。
- **鉴权（§9.6）**：`ServerState` 增 `acl` 字段与 `with_acl`（`new` 保持无 ACL 兼容）。`authenticated_principal` 恒时遍历令牌反查参与者；`authorize_action`（动作端点：令牌归属 + 实例授权）、`authorize_key`（生命周期 / 查询 / compact / 读审计）、`authorize_domain`（`:analyze` 该域任一授权）。`GET /domains` 逐实例过滤并剔除空域。无令牌 `401`、越权 `403`（结构化错误体含 `domain` / `instance`）。覆盖端点：load / instantiate / unload / compact / `:analyze` / `:compose`、seed / produce / read / consume / gates / enabled、counter / archive / status / events、read-audit。
- **读审计（pact-store + pact-engine）**：`ReadAuditRecord{version, ts, domain, instance, participant, token_id}`；`Store::{append_read_audit, read_audit_tail}`（append + `sync_all`；尾部 N 条按时间正序；文件不存在 → 空；仅容忍末尾无换行的崩溃半行，其余坏行 fail-fast）。`EngineConfig.read_audit`；Rule-R 成功后写审计，`Store` 错误经 `EngineError::Store` 上抛 → REST `500`（fail-closed）。新增 `Command::QueryReadAudit` + `Engine::read_audit(key, limit)`。
- **接口面**：`GET /domains/{d}[/instances/{i}]/read-audit?limit=`（缺省 100、夹取 `[1, 1000]`）→ `{records:[...]}`；`pact_http::client::Client::read_audit(...)`。审计流不产生事件、不推进 `seq`、不参与重放 / Rule-T / 快照 / 压缩；实例未加载也可查询。
- **测试**：新增 10 项——config 3（ACL 无令牌拒绝、条目格式校验、通配/实例匹配）、store 3（append+tail 且独立于事件日志、末尾半行容忍、跨块反向扫描）、engine 2（Rule-R 审计字段 / limit / 重启持久化 / 不改事件日志、审计写失败 fail-closed 且关闭时不阻断）、http e2e 2（401/403/通配/列表过滤/查询与审计视图受控/卸载；审计视图记录 / limit / 实例隔离）。全仓 **155 项**全绿（基线 145 + 10）；fmt/clippy 清零。
- **形式层不变**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动；`docs/pact*` 母本未动。CLI/MCP 未新增子命令（只读视图走 REST/client）。
- **安全审视待办**：提交/推送前按流程做一次全面安全审视。

## 2026-10-20（第 25 轮，推送前全面安全审视）

- **审查范围**：本轮全部改动（`pact-store` 版本化与压缩、`pact-engine` 快照锚与 Compact、`pact-http` 端点与压缩配置、`pact-cli`/`pact-mcp` 入口、文档）。
- **核对项与结论（零可利用漏洞）**：
  - **路径穿越**：`compact` 端点键与既有端点同经 `InstanceKey::validate()`（谓词 `[A-Za-z0-9_-]{1,128}`，天然禁 `..` 与分隔符），存储层路径构造仍集中在 `log::events_path` / `domain_dir`；新增 `logmeta_path` 同源复用同一 `(domain, instance)` 参数与校验，无新增可达的穿越面。
  - **SQL 注入**：`prune_events` 为 `DELETE … WHERE domain = ?1 AND instance = ?2 AND seq <= ?3` 全参数化绑定，无字符串拼接。
  - **内存安全**：全仓 `unsafe` 零出现；新增代码不含 `unsafe`、不含 `unwrap()` 于外部输入路径（仅锁中毒 `.expect()` 与测试）。
  - **崩溃原子性与数据丢失**：压缩五步序中「快照（提交点）→ logmeta 水位 → 日志 tmp+fsync+rename → 索引裁剪」保证任一中间点崩溃均落到两个一致态；`SnapshotRequiredForCompactedLog` 护栏防「日志已截断而快照缺失」的静默丢数据；索引自愈路径（`base_seq > 0` 补 `prune_events`）不删 tokens 投影。
  - **版本兼容**：读到更高 `version` 一律 fail-fast（日志/快照/logmeta 三处），不静默按 v1 重放未知语义；缺 `version` 才按 v1 兼容。
  - **拒绝服务面**：`keep_last` 为 `u64`，计算用 `saturating_sub`，无溢出/下溢；`/compact` 为 O(日志大小) 写放大操作，单个实例 flock 互斥故不可并发放大；`/health`/`/ready`/`/metrics` 及 `/compact` 均不参与令牌比较路径，不构成恒时比较旁路。
  - **信息泄露**：新增错误变体（`UnsupportedSchemaVersion`、`SnapshotRequiredForCompactedLog`）的 HTTP 映射只回显版本号/`base_seq` 等非敏感结构量，不额外暴露服务器路径（沿用既有 `Store` 500 通道）。令牌比较仍走恒时 `ct_eq`。
- **发现并处置（均为「补文档/补告警」，非代码缺陷）**：
  1. **压缩端点不校验令牌**（认证边界问题）：`POST .../compact` 与域生命周期端点一致不绑定参与者身份，且属**破坏性**操作。处置：在 `manual.md` §2.3.1/§7.4/§8.2 明确「令牌只覆盖四类动作端点，daemon 不得暴露给不可信网络，域级 ACL 属子项 (3) 未实现」，并在 `architecture.md` §5 安全边界行同步登记。
  2. **`compact_on_start` 静默丢弃审计前缀**：`compact_on_start = true` + `compact_keep_last = 0` 时每次重启都会丢弃全部已入快照前缀。处置：daemon 启动打印显式告警，manual §7.4 记入风险。
- **未处置（受限于范围，登记为已知限制）**：压缩端点无速率限制/并发配额（单写者锁已兜底）；域级 ACL 仍缺（D25 子项 3）。

## 2026-10-20（第 24 轮，D25 子项 1：日志压缩与 schema 版本化实施）

- **四项设计取舍（用户裁决，均选推荐项）**：压缩触发 = **显式运维接口 + 可选启动自动**；截断机制 = **前缀删除 + 旁路 base_seq 清单**；幂等表 = **纳入快照**；version 形态 = **每事件顶层 `version` 字段**。
- **schema 版本化（pact-store）**：新增 `WireEvent{version, #[serde(flatten)] event}` 与 `EVENT_SCHEMA_VERSION = 1`（唯一真源）。日志与线上格式统一经 `to_wire_json` 输出（实测行首为 `{"version":1,…}`）；读取经 `into_event()`，`version > 当前` ⇒ `StoreError::UnsupportedSchemaVersion` fail-fast；缺 `version` 由 serde `default` 视为 v1（旧日志/旧快照零迁移）。`Notification`（SSE）同步加顶层 `version`。
- **前缀压缩（pact-store）**：`Store::compact(key, snap, keep_last, compacted_at)` 五步序——① `write_snapshot`（**提交点**）② `base_seq = max(snap.seq - keep_last, prev_base)` 写 `<instance>.logmeta.json`（先于日志重写，水位**单调不回退**）③ `rewrite_retaining` 原子重写日志（保留 `seq > base_seq` 行原始字节，丢弃尾部半行，tmp+fsync+rename）④ `IndexStore::prune_events` 裁剪 `events` 行 ⑤ 移除并重建追加句柄，`last_seq = snap.seq`。返回 `CompactionResult{snapshot_seq, base_seq, retained, discarded}`。
- **快照自足锚（pact-store）**：`Snapshot` 扩为 `{version, seq, dl_seq, declaration, state, consumed_requests}`（`Snapshot::new(...)` 统一构造）。`consumed_requests` = Rule-E 幂等表（`request_id → (TokenId, ArchiveEntry)`）——压缩丢弃 `Consumed` 行后跨重启幂等仍生效。
- **恢复分支（pact-store + pact-engine）**：`read_and_sanitize(path, base_seq)` 的 seq 校验由「首行恒为 1」放宽为**连续性**（首行允许 ≤ `base_seq+1` 的未截断超集态，其余逐行 +1）。`Store::recover` 读 logmeta → 载快照 → 若日志无 `DomainLoaded` 则要求快照覆盖 `max(base_seq, first_seq-1)`，否则 `SnapshotRequiredForCompactedLog` fail-fast；`last_seq = max(日志末尾, base_seq, snap_seq)`；`base_seq > 0` 时补 `prune_events` 自愈崩溃窗口。索引一致性校验放宽为「索引不落后于日志」（`idx_last == log_last || (log_last == 0 && idx_last <= base_seq)`），避免误触发全量 `rebuild` 连带清空 tokens 投影。
- **引擎（pact-engine）**：`recover_instance` 新增 `snapshot_is_anchor` 分支（日志无 `DomainLoaded` ⇒ 以 `snap.declaration`/`snap.dl_seq`/`snap.state`/`snap.consumed_requests` 为基补尾部；无快照 ⇒ `EngineError::Recovery`）；幂等表基线 `idem_floor = base_seq`（快照锚）或 `dl_seq`（日志锚）。所有快照构造点统一走 `snapshot_of(&InstanceSession)`，保证恒自足。新增 `Command::Compact`、`Engine::compact(key, keep_last)`、`EngineConfig{compact_on_start, compact_keep_last}`；`Engine::spawn` 保持默认配置兼容，新增 `spawn_with_config`。恢复末尾按配置逐实例自动压缩（单个失败仅 `eprintln!` 告警，不阻断启动）。
- **接口面**：REST `POST /domains/{d}/compact` 与 `/domains/{d}/instances/{i}/compact`（`CompactReq{keep_last?}` → `CompactionOutcome{domain, instance, snapshot_seq, base_seq, retained_events, discarded_events}`）；CLI `pact compact --domain D [--instance I] [--keep-last N]`（离线 `replay` 改经 `WireEvent` 校验版本）；MCP `pact_compact`（工具数 16 → 17）；`pact.toml` 新增 `compact_on_start`（缺省 false）与 `compact_keep_last`（缺省 0），`daemon.rs` 经 `spawn_with_config` 传递。
- **取舍与边界**：压缩后被丢弃前缀不再可查（审计视图 = 快照 + 保留尾部事件；`events` 索引行被裁剪、`tokens` 投影行保留）；v1 不做自动周期性轮转，压缩为运维显式动作。`SeqGap` 的既有语义（中部缺行 fail-fast）不变。
- **测试**：新增 14 项——store 单测 6（version 兼容/未来版本致命、截断后 base_seq 校验、logmeta 往返与缺失、compact 截断/keep_last/无快照 fail-fast）+ 既有 store 快照测试改造 + engine 集成 3（压缩保持状态与幂等、keep_last 保留尾部 + 线上 version、启动自动压缩）+ http e2e 2（压缩截断 + 幂等跨重启 + 404）+ CLI/MCP e2e 断言扩展。全仓 **145 项**全绿（基线 131 + 14）；fmt/clippy 清零。
- **形式层不变**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` **零改动**；`docs/pact*` 母本未动。

## 2026-10-20（第 23 轮，D25 子项 2：运维接口实施）

- **范围**：仅 D25 子项 (2)。(1) 日志压缩/schema 版本化、(3) 域级 ACL/审计读视图未动（均须按域实例双键重述）。
- **新增 §9.5 运维接口**：`GET /health`（存活，不依赖引擎）、`GET /ready`（就绪，探针 = 一次 `list_domains` 往返，不可应答 → 503）、`GET /metrics`（Prometheus 文本格式，`text/plain; version=0.0.4`）。指标：`pact_domains_declared`、`pact_instances_loaded`、`pact_counter_size{domain,instance}`、`pact_archive_size{domain,instance}`、`pact_terminated{domain,instance}`、`pact_uptime_seconds`；仅统计**已加载**实例（`InstanceNotFound` 跳过）。
- **认证口径**：三端点**免令牌**（用户裁决）。理由：探活/抓取惯例；部署默认 unix socket，TCP 建议 `127.0.0.1`。域操作端点认证不变。
- **PID 文件**：`Config.pid_file: Option<PathBuf>`（缺省不写，向后兼容）；启动写入自身 PID、退出（含信号路径）drop 删除；启动时陈旧文件且 PID 存活 → `AlreadyExists` 拒绝。Linux 以 `/proc/<pid>` 判定存活（PID 复用假阳性 → 保守拒绝），非 Linux 交由 `pactd.lock` flock 兜底。
- **优雅停机**：`pact-http::server` 引入 `watch` 停机广播 + `ConnTracker`（`AtomicUsize` + `Notify`）在途计数。SIGINT/SIGTERM 同路径 → 停止 accept → 有界（`DRAIN_TIMEOUT` = 10s）等待在途归零 → `engine.shutdown()` 终态快照 → 清理 PID/套接字。**有界是必须的**：SSE 长连接会占满窗口。监听循环签名增加 `watch::Receiver<bool>` 与 `ConnTracker`（5 处测试调用点同步）。
- **接口形状**：`ServerState` 增加 `started_at: Instant`（uptime 基准）；`text_response` 显式设 `Content-Type`。
- **测试**：新增 8 项——daemon 单元 3（PID 写入/删除、陈旧检测、未配置不写）+ daemon_e2e 3（免令牌、指标含已加载实例、停机停止 accept 且 drain 归零）+ 进程级 2（SIGTERM 优雅退出并清理 PID/套接字、陈旧 PID 拒绝启动）。全仓 **131 项**全绿（基线 123 + 8）；fmt/clippy 清零。
- **形式层不变**：`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` / `pact-store` / `pact-engine` **零改动**；`docs/pact*` 母本未动。

## 2026-10-20（第 22 轮，过程产物移除与阶段 8 排期重挂）

- **触发**：用户「移除这两个文件，然后继续开发」。阶段 10 交付后，模型 C 评估与咨询的**过程产物**已并入正式文档，不再需要留存。
- **移除**：`model-c-impact.md`、`model-c-expert-brief.md`（根目录临时产物，此前随 `a65ce34` 入库）删除。
- **悬空引用消解（仅正文指向，未改结论）**：`design.md` D27 行末的「评估与执行序见 …」与 §11 阶段 8 行的「（见 `model-c-impact.md` §6）」改写为自足表述——阶段 8 与 D27 的耦合点（(1) 压缩部分、(3) 全部按双键重述、`:compact` 路由随实例段重设计）已内联在该行。
- **排期**：阶段 8（D25 生产化三项）解除暂停（其暂停原因即与实例化耦合，现已随阶段 10 消除），成为下一实施目标；阶段 8 行状态仍为「未开始」。
- **零代码改动**：本轮不触碰任何 crate；`docs/pact*` 母本未动。

## 2026-10-20（第 21 轮，术语精化：「域实例」）

- **触发**：用户指出上一轮 G2 把「域」改为「实例」的措辞会与工程上的「实例」混淆，且不排除将来细化出更多类型-实例分离（如分布式 counter），要求统一改为「域实例」。
- **术语定案**：中文文档/注释中，指 D27 实例键的概念一律写「**域实例**」——即论文 `Counter instance`（§1.3 描述性短语），**与论文已废弃的英文 `domain instance` 无关**，亦不替代论文术语；映射登记于 `design.md` D27「术语登记」。论文母本 `docs/pactv1_rev3.md` 未触碰。
- **改点（正文措辞，零语义）**：`design.md` D23（2 处）、§1 crate 注释、§6.3（411）、§7.1（445）、§8.1（521/523）、§8.2（527）、§8.3（531）；`requirements.md` FR9/FR10/FR12/FR15/NFR2；`manual.md` Rule-T 行；`architecture.md` §2.3 启动恢复步骤。**有意保留**：论文条款名「单实例作用域」及 G1 CPN 行的论文锚定表述（保持与论文一致，不用中文「域实例」）。
- **代码侧（无行为变化）**：注释口径「每实例/同一实例/逐实例/不锁定实例」→「每域实例/同一域实例/逐域实例/不锁定域实例」——`pact-store/src/{log,event,store}.rs`、`pact-engine/src/{engine,lib,notify}.rs`、`pact-http/src/dto.rs`；另修 `pact-core/src/domain.rs` 中误用「域实例」指 **Domain 声明**的文档注释 → 「域声明」，消除与中文「域实例（= 实例键）」的撞义。
- **形式层语义不变**：`pact-parser`/`pact-analysis`/`pact-compose` 零改动；`pact-core` 仅 `domain.rs` 一处文档注释措辞（无类型/无行为变化）。
- **验证**（2026-10-20 收尾重跑）：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零告警、`cargo test --workspace` **123 passed / 0 failed**。

## 2026-10-20（第 20 轮，多实例约束覆盖核对后的文档补齐 G1/G2/G3）

- **触发**：见 `docs/requirements-change.md` 第 12 轮。核对基准 = rev3 与多实例直接相关的条款（§1.3 两半、§1.4.1 唯一性、Rule-T scope note、§4.5/§B.2 CPN scope、§1.8 及 §4.1–4.3 术语）；结论：C1/C2/C3/C6 全覆盖，C4/C5 未登记（G1）、幂等作用域措辞滞后（G2）、母本 §1.4.1 Rationale 与 Definition 措辞不一致（G3）。
- **G1（补登记）**：`design.md` 决策 **D12** 行与 §11 **阶段 6** 行补入论文 §4.5/§B.2 的 **单实例作用域**约束——每个 Counter 实例独立编码、库所与迁移不跨实例共享、跨实例协同不入 `N(D)`；作为 `pact-cpn`（阶段 6，未开始）的实现前置条件。
- **G2（措辞收窄）**：把「以域名为会话键」的残留文本统一收窄为「以实例为会话键」（D27/FR19）。改动点：决策 **D23**（「同域其他行」「同域内重复 `request_id`」）、§6.3 幂等段（411 行）、Consumed 自足段（489 行）、§8.1 启动恢复（518 行「逐域取最后一次 DomainLoaded」）与 Rule-E 处理顺序（520 行「同域 `request_id`」）、§8.2 通知（524 行「每域一个 broadcast」）、§8.3 Rule-T（528 行「不锁定域」）、§1 crate 注释（48 行「多域会话」→「多域/多实例会话」），以及 **§7.1/§7.2 存储布局**修正为 D27 布局（`<domain>/<instance>.events.jsonl` + `<instance>.snapshot.json` + `declaration.pact` + `pactd.lock`；`seq` 明确为**每实例**单调）。
- **G3（母本就地修订，不新增版本）**：经用户**特批**，就地修订 `docs/pactv1_rev3.md` §1.4.1 的 **Rationale**——原文 "tokens carry globally unique IDs" 与同节 Definition（"`TokenId` is unique within its Counter; implementations may strengthen this to cross-instance uniqueness…"，121 行）不一致，改为与 Definition 一致（保留「实现可加强为跨实例唯一」的 Q4 口径）。**不产生修订版本**：仍为 rev3（文件就地修改，不新建 rev4、不改任何 rev3 指针）；rev2→rev3 **差异块由 9 增至 10**（新增该 1 处纯措辞一致性修订，零语义）。
- **代码侧（无行为变化）**：仅修正 6 处**文档性注释**口径（`pact-store/src/log.rs`、`pact-store/src/event.rs` ×2、`pact-engine/src/notify.rs`、`pact-engine/src/engine.rs` 配置项注释、`pact-http/src/dto.rs`）——「每域」/「同域」→「每实例」/「同一实例」。无代码语义改动。
- **形式层不变**：`pact-core`/`pact-parser`/`pact-analysis`/`pact-compose` 零改动；本轮仅文本与注释。
- **验证**（2026-10-20 收尾重跑；沙箱宿主链接器方案）：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零告警、`cargo test --workspace` **123 passed / 0 failed**（与阶段 10 基线一致）。

## 2026-10-20（第 19 轮，阶段 10 实施：域双键实例化落地，路径 β）

- **触发**：用户「按计划实施」——实施阶段 10（`docs/design.md` §11 行 10，D27 路径 β）。**范围仅 D27 双键本体**：存储双列 + 域子目录、引擎声明注册表 + 实例会话、接口全链、通用护栏；**阶段 8 仍暂停**（schema 版本化、日志压缩、health/metrics/PID、ACL/审计读视图本轮不做）。
- **D27 行更新**：把「派生待定」节替换为「派生细节定案 + 已实现（见 §9.5 / §11 行 10）」——旧日志 fail-fast、实例键路径段承载、`DomainLoaded` 不新增 `instance`、声明原文另存 `declaration.pact`、护栏 flock + `DomainBusy`。
- **实现决策（本轮推导 R1–R10）**：
  - R1 新增 `InstanceKey{domain, instance}`（定义于 `pact-store`，engine 复用；`Display = "domain/instance"`）；
  - R3 `DEFAULT_INSTANCE = "default"` 常量置于 `pact-engine`；
  - R4 `pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` **零改动**（形式语义不变）；
  - R5「实例非空」判据 = `last_seq > dl_seq`（`DomainLoaded.seq`）；
  - R6 `POST /domains` = declare + instantiate `default`；R7 `GET /domains` 响应由 `Vec<String>` 改 `Vec<DomainListing{name, instances}>`；
  - R8 recover 顶层遗留旧文件 → `LegacyLayoutDetected` fail-fast；R9 flock 独占锁落在 `pact-http::daemon` 启动路径（新依赖 `fs4`）；
  - R10 引擎内部 `declare()` / `instantiate()` 私有方法 + 公开命令 `LoadDomain` 与新增 `Instantiate`。
- **逐层落地**：
  - **pact-store**：`key.rs`（`InstanceKey` + `validate`）、`error.rs`（`InvalidInstanceName` / `LegacyLayoutDetected` / 共享谓词 `valid_key_name`）、`log.rs`（`domain_dir` + 三参 `events_path`/`snapshot_path` + `declaration_path`）、`store.rs`（重写：`logs: Mutex<BTreeMap<InstanceKey, (EventLog, u64)>>`、`RecoveredInstance{key, events, truncated}`、先探测旧布局再两层扫描、`write_declaration`/`load_declaration`、`last_seq(&InstanceKey)`、`list_instances`/`list_domains`）、`index.rs`（`events(domain, instance, …, PRIMARY KEY(domain,instance,seq))`、`tokens(token_id PK, domain, instance, …)`；`open()` 自愈：无 instance 列则 DROP 重建）。
  - **pact-engine**：`declarations: BTreeMap<String, DeclarationEntry>` + `instances: BTreeMap<InstanceKey, InstanceSession>`；`InstanceSession{declaration, domain, state, last_seq, dl_seq, events_since_snapshot, terminated, consumed_requests, notify}` + `is_occupied()`；`EngineError` 删 `DomainNotFound`、增 `InstanceNotFound`/`DomainNotDeclared`/`InstanceExists`/`DomainBusy`；facade 11 动作方法收 `key: InstanceKey`、新增 `instantiate`/`list_domains`、`load_domain` 增 `instance`/`force` 并先做 `validate_domain_name` + `validate_instance_name`（防路径穿越）；`views.rs` 增 `DomainSummary.instance` 与 `DomainListing`；`notify.rs` `Notification.instance`（非 Option）。
  - **pact-http**：路由增 `POST /domains/{d}/instances` 与 `/domains/{d}/instances/{i}` 任何段 + 实例级镜像组；所有 handler 抽 `*_inner(&ServerState, key, …)` + default/instance 双薄 wrapper（auth 统一在 `*_inner`，实例路由同受令牌约束）；`dto.rs` 增 `LoadDomainReq.instance/force`、`InstantiateReq`、`DomainsResp → Vec<DomainListing>`、`DomainInfo.instance`；`client.rs` `base_path(domain, instance)`（None/`default` ⇒ 默认族）+ 22 方法加 `instance` + `instantiate`；`daemon.rs` 增 `acquire_data_dir_lock`（`<data_dir>/pactd.lock`）；`From<EngineError>` 增 `InvalidDomainName`/`InvalidInstanceName` → 400（原落 500）。
  - **pact-cli**：`DomainCmd::Load` 加 `instance`/`force`；新增 `Instantiate`/`Instances`；`Info`/`Unload`/Seed/Produce/Read/Consume/`CounterCmd::Ls`/`ArchiveCmd::Ls`/`GatesCmd::Show|Enabled|Status` 加 `--instance`；`list` 打印 `name [instances…]`。
  - **pact-mcp**：15 → **16** 工具，新增 `pact_domain_instantiate`；`LoadIn.instance/force`；各 In 加 `#[serde(default)] instance: Option<String>`；`ComposeIn` **移除** `instance`（compose 只作用 default，消除 clippy dead_code）。
  - **pact-runtime**：`ParticipantClient` 增 `fn instance()`；`RestClient`/`McpClient` 增 `instance` + `with_instance`；新增自由函数 `bound_ready(listings, domain, instance)`；`drive()`/`executor` 域发现改用 `bound_ready` + `list_domains`；`Orchestrator` 增 `instance` + `with_instance`；bin `FileConfig` 增 `#[serde(default)] instance`。
- **测试（FR13 四层）**：改造既有锚点（`engine_integration.rs` 全文重写、`daemon_e2e.rs`、`cli_e2e.rs`、`mcp_e2e.rs` 工具数 15→16、`unattended_e2e.rs`、`runtime_e2e.rs`）+ 新增 β 专项：engine 集成 3（`same_declaration_supports_multiple_instances` / `replacing_declaration_does_not_disturb_running_instance` / `declaration_backfill_supports_new_instances_after_restart`）；daemon e2e 2（`instance_routes_are_isolated` / `invalid_key_names_rejected_without_touching_fs`）；cli e2e 1（`instance_commands_and_default_fallback`）；runtime e2e 1（`instance_binding`，两实例并发自驱）；daemon 单元 1（`data_dir_lock_is_exclusive`）。
- **两处安全修复（详见「安全审视」）**：① flock 护栏因 `fs4::fs_std::FileExt::try_lock_exclusive` 返回 `Result<bool>` 而被 `Ok(false)` 静默放行 → 改为检查 bool；② 非法域名/实例名原落 HTTP 500 → 加 `From<EngineError>` 映射到 400 `InvalidDomainName`/`InvalidInstanceName`。
- **验证输出摘录**（沙箱，宿主机工具链包装 `/tmp/hostlink`）：
  ```
  cargo fmt --all --check                               → 无输出（FMT_OK）
  cargo clippy --workspace --all-targets -- -D warnings → 零警告
  cargo test --workspace                                → TOTAL_PASSED=123（基线 110 + β 专项，全绿）
  ```
- **文档**：`requirements.md`（FR19 细化 + 实现状态）、`design.md`（D27 行、§9.2 实例路由 + 错误码小节、§9.3 CLI 实例命令、§11 阶段 10 状态 → 已完成）、`architecture.md`（§1 图：引擎声明注册表 + 实例会话表 / 存储域子目录布局；§2.3 启动恢复；§5 多实例行 → 已实现 + 存储布局定案）、`manual.md`（§7.1 存储布局、§8.1 CLI 表、§8.2 REST 表、§8.3 MCP 16 工具、§9.4 错误码、§10 故障排查、§11 限制 9/10）、四份 change 文档。
- **规范母本未触碰**：`docs/pact*`（rev1/rev2/rev3）全程未改；本轮为实现落地，不新增范式语义（不触发边界判据 4）。
- **沙箱构建偏差**：沙箱 rootfs 无链接器/C 编译器，构建测试经宿主机 `/run/host` 工具链包装（`/tmp/hostlink`，`ld`→`ld.bfd`）完成，**未安装任何包、未改 /usr**（同第 15/17 轮做法）。

## 2026-10-20（第 18 轮，域双键实例化的形式层裁定与论文 rev3 切换）

- **触发**：用户提问「两个同一个域的项目并行如何处置？」→ 指示「暂停实施阶段 8，评估模型 C 的全面影响，包括论文（可能需要领域专家）」。评估产出根目录 `model-c-impact.md`（五层 × 三档判定 + 阶段 8 交叉标注 + 专家问题清单）。
- **触发事实（评估核出）**：域名今天**既是定义标识又是会话键**——`LoadDomain` 为替换语义（同名额重置会话、清快照、seq 续接），两个并行项目共用同一域名时第二次 load 会**销毁**第一本的 Counter 与快照；域名直接作文件名与 REST 路由键；`{d}:analyze`/`{d}:compose` 已占用 `:suffix` 命名空间；全仓无 `data_dir` 锁。
- **领域专家裁定**（咨询简报 `model-c-expert-brief.md`，答复全文带回）：Q1 **(a)** 概念承认——以 §1.3「实现自由」形式落地；Q2 **(a)** 术语消歧——指「声明/组合结果」的 `domain instance` 统一改称 `domain declaration`；Q3 **(a)+(c)** 限定重述——§1.3 集中说明 + Rule-T 增 scope note；Q4 保留「globally unique」但**作用域收窄至该 Counter**，实现以 ULID 加强为跨实例唯一；Q5 确认**无任何论文级不变量或定理依赖「一个域名至多一本 Counter」**；Q6 确认 CPN 编码需加单实例作用域说明；Q7 **不新增正式术语**（工程侧用描述性短语 `Counter instance`）。总括口径：不涉 SOS 规则/GateHolds/GateExpr/良构约束/静态分析算法的语义修改；实例划分属实现自由**不越界**；母本修订须用户显式确认。
- **用户裁决**：破例授权改论文为 rev3；**立项模型 C**；实现走**路径 β**（`pact-store`/索引显式 `(domain, instance)` 双列 + 引擎**声明注册表**，`declare` 与 `instantiate` 分离）。
- **论文 rev3 落地**（新增 `docs/pactv1_rev3.md`，`pactv1_rev2.md` 留作历史）：5 类改动共 **9 个 diff 块**——§1.3 增列「Counter instantiation and partitioning」条款；§1.4.1 定义段重写（TokenId 作用域）；Rule-T 后增「Scope note」；§4.5 与 §B.2 各增 CPN 单实例作用域说明；§1.8 定理行及 §4.1/§4.2/§4.3 四处 `domain instance` → `domain declaration`。**`diff -u` 核对结论：仅此 9 处增补/替换，零文法变化、零语义变化、零行丢失**（§x.y 编号不变，全部既有引用仍有效）。
- **母本引用同步**：`design.md` / `requirements.md` / `manual.md` / `skill-adaptation.md` / `examples/README.md` / `examples/README.zh-CN.md` 的 `docs/pactv1_rev2.md` 指针全部改为 rev3（历史变更文档中的 rev2 记述保留不动）。
- **文档落地**：`design.md` 新增 **D27**（域双键实例化与形式层裁定：术语登记 + 三项待定细节）＋ §11 新增**阶段 10**、阶段 8 验收注明与实例化耦合；`requirements.md` 新增 **FR19 域双键实例化**、FR10 多域会话行补实例维度、NFR7 补多实例边界（同 `data_dir` 须 flock 保证单写者）。
- **零代码改动**：本轮全程未改任何 crate；`pact-core` 纯度白名单不变；**阶段 8 实施保持暂停**（其三项中 (1) 压缩与 (3) ACL/审计与实例化强耦合，(2) 的 `:compact` 路由与既有 `:suffix` 分发冲突，须随双键一并重设计）。
- **待裁决（D27 细节 3 项，用户）**：存储布局（单层文件名 vs 域子目录）、默认实例名取值、实例与声明版本的绑定（创建时快照声明 vs 引用最新）。

## 2026-10-19（第 17 轮，阶段 7 `pact-runtime` 落地）

- **触发**：用户批准实施阶段 7（`docs/design.md` §11 行 7；D24），按 `spec.md`/`tasks.md`（**临时规划产物**，实现并更新 `docs/` 后删除）执行；硬约束为 pact-core / pact-store / pact-engine / 三接口 **零改动**。
- **新增 crate `crates/pact-runtime/`**（lib `pact_runtime` + bin `pact-runtime`，根 `Cargo.toml` members 追加）：
  - `src/client.rs`：`ParticipantClient` trait（`Send + Sync`）、`WakeStream`、`Endpoint`（`Unix`/`Tcp`）——传输层抽象；
  - `src/rest.rs`：`RestClient` 包装 `pact_http::client`；`src/mcp.rs`：`McpClient`+`EventNotifier`（rmcp **stdio 子进程** MCP 客户端，镜像 `unattended_e2e` 用法）；
  - `src/driver.rs`：`Bootstrap`/`DriverConfig`/`AgentReport`/`absorb`/`drive()`——P1（生产优先）/P2（汇消费）/P3（等待）策略、订阅推送 + 轮询兜底、`pristine` 守卫、500ms 终止宽限，与 `unattended_e2e` **逐条同构**（差异仅"错误以 `Result` 返回"而非 panic）；
  - `src/orchestrator.rs`：`Transport`/`AgentSpec`/`BootstrapSpec`/`Orchestrator::run()`——每 agent `tokio::spawn`，未来对象 `Box::pin` 统一（REST/MCP 两具体 client 不引入 dyn）；
  - `src/executor.rs`：`ExecutorConfig`/`Compensation`/`SideEffectHandler`/`SideEffectExecutor`/`StopSignal`——订阅 `Consumed` 通知、archive 游标增量处理、失败以 **Rule-S 追加补偿事实**（saga），不表现为消费回滚（FR15/D23 边界）；
  - `src/bin/pact-runtime.rs`：TOML 配置驱动的生产编排入口（装配编排 + 可选执行器，输出各 agent 报告 JSON）。
- **依赖方向落地**：`runtime → pact-http`（类型化 REST client）+ `pact-engine`（线上视图 `CounterView`/`StatusView`/`GateClause`/`Notification`）+ `pact-core`（`Token`/`ArchiveEntry`/标识符 newtype）；`pact-mcp` 仅作可执行文件由编排 spawn、**不**作 lib 依赖（避免给 `pact-http` 扩大公共 re-export 面，spec D-A）。engine/store/core/三接口 **零改动**（`git status` 核对）。
- **两处修复**：
  - ①**执行器域发现**：编排与执行器并发启动时，执行器可能先于引导者装载域而收到 `DomainNotFound` → 执行器增**域发现轮询**（对齐 `drive()` 的 `list_domains` 轮询再 `subscribe`）；
  - ②**驱动 P1 需读见证**：P1 对每个见证前驱先 `read()` 再生产，测试域 `worker` 补 `reads task` 门（P1 见证可满足性前提）。
- **测试**：新增 5 项——集成 1（`tests/runtime_e2e.rs`，单 `#[tokio::test]`(multi_thread) 内起 **in-process daemon**（`Store::open`+`Engine::spawn`+`ServerState`+`router`+`serve_unix`）顺序跑：REST 编排 SE 生命周期（Σ 产 4/消 5/读 4、溯源链、`max_counter ≤ 5`、终态 `counter_size:0/archive_size:5/terminated:true`）｜REST 编排 ReAct 循环（7 产/8 消/7 读、`max_counter ≤ 2`、archive 序两轮同构）｜副作用执行器（失败 → 补偿事实 `task_failure` 由 `materializer` 追加、成功计数、游标无重复））+ 单元 4（`absorb` 计数/`max_counter`/`notified_terminated`；`effect_types` 过滤；`Endpoint::args` 生成；`RuntimeError::Display` 与 `From<ClientError>` 映射）。MCP 路径代码镜像已验证可编译的 e2e 用法，不在测试内 spawn（spec D-D：跨 crate 取不到 `CARGO_BIN_EXE_pact-mcp`）。
- **验证输出摘录**（沙箱，宿主机工具链包装）：
  ```
  cargo fmt --all --check                              → 无输出（FMT_OK）
  cargo clippy --workspace --all-targets -- -D warnings → Finished `dev` profile … in 1.20s（零警告）
  cargo test --workspace                                → TOTAL_PASSED=110（基线 105 + 本轮 5，全绿）
    Running tests/runtime_e2e.rs  test result: ok. 5 passed; 0 failed; …
  ```
- **文档**：`docs/design.md`（D24 行补"已实现"+依赖细化；§1 crate 表去"未实现"、依赖行改 `runtime → http/engine/core` 并注明 `pact-mcp` 仅 bin；§11 阶段 7 状态 → 已完成并附验收摘要；§13 L3 与构建形态措辞更新）；`docs/architecture.md`（§6 标题去"未实现"+新增"落地形态"条+依赖方向细化；§5 层边界行更新）。`docs/requirements.md`/`docs/manual.md` **不改**（FR16 已存在）。
- **规范母本未触碰**：`docs/pact*` 全程未改；D24 为**实现层**生产运行层，不新增范式语义（不触发边界判据 4）。
- **执行偏差**：沙箱 rootfs 无任何链接器与 C 编译器，构建/测试经宿主机（`/run/host`）工具链包装（`/tmp/hostlink`，gcc/ld→ld.bfd/ar/…）完成，**未安装任何包、未改 /usr**；沙箱与宿主 glibc 同版，链接产物在沙箱内直接运行。命令语义与 CI 一致（同第 15 轮做法）。
- **安全审视**：新增为客户端/编排/执行器代码——仅经 `pact_http::client` 与 `spawn` 子进程连接既有 daemon；无 `unsafe`；无新增密钥（bearer 由配置注入，不硬编码）；`McpClient` 的 `spawn` 仅拉 `pact-mcp` 可执行文件（路径来自配置）；无新依赖引入除 workspace 既有项之外（`toml`/`async-trait` 等已在 workspace）；无文件路径注入（配置项仅作连接参数与类型名）。无新增攻击面。

## 2026-10-19（第 16 轮，生产化边界裁决：D24–D26）

- **触发**：用户就"生产系统功能边界"提问并四问裁决（详见 `docs/requirements-change.md` 第 9 轮）。
- **新增 D24 生产运行层归属**：本仓库新增 crate（暂名 `pact-runtime`），承载参与者驱动循环（订阅/轮询 → `my_gates`/`enabled` → 决策 → 行动）+ 多 agent 编排 + 外部副作用执行器（outbox/saga 消费端）；引擎保持被动（D5）、MCP 维持 stdio-only（D20）均不变。
- **新增 D25 引擎生产化三项**：事件日志压缩与 schema 版本化（pact-store）；运维接口 health/ready + metrics + PID 文件 + 信号化优雅停机（pact-http/engine）；域级授权与审计读视图（pact-http 接口层，读审计**可选开启、默认关闭**——落地 D9 保留的审计扩展位，修订其默认值表述）。
- **新增 D26 分发产物归属**：本仓库 `dist/`（Dockerfile、systemd unit、musl 静态打包脚本），不接入 CI 语义测试（同 D22）。
- **结构更新**：§1 crate 划分增 `pact-runtime`（标注未实现）与其依赖方向（`runtime → mcp / http`，不反向）；§11 实现阶段增阶段 7/8/9（均"未开始"）；新增 §13「生产运行层与生产化边界」（含边界判据四条与构建形态）；§12 补 MCP 提供形态确认。
- **代码/测试零变更**：本轮纯文档与设计裁决；workspace 105 项全绿结论沿用第 15 轮。
- **规范母本未触碰**：`docs/pact*` 未改；本裁决为实现层工程边界，不涉及范式语义（不触发边界判据 4）。
- **安全审视**：本轮仅新增文档，无网络、无密钥、无 `unsafe`、无依赖变更。**登记待办风险**：FR17 的域级 ACL 与读审计属访问控制面，实现时须单独做安全设计（不属本轮）。

## 2026-10-18（第 15 轮，消耗提交契约与 Rule-E 幂等：发现 A/B 修复 + D23）

- **触发**：用户就"消耗原子性"组织评审后裁决——修复评审发现 A/B，采纳"文档化实现契约 + 补幂等 + witness-token 示例（2a）与 outbox 语义（2b）并行"组合。
- **发现 A（高）**：`Event::Consumed` 只记 `token_id`，重放时从 Counter 反查 token（`StoreError::ConsumeTargetMissing`）——单行日志不自足，与 NFR2"日志自身即完整审计记录"不符。**修复**：`Consumed` 改携 `token` 全量快照（`exhausted_at`=`ts`、`exhausted_by`=`by`）+ 可选 `request_id`；`apply_event` 与索引 `rebuild` 直接取事件内 token，删除 `ConsumeTargetMissing` 变体。
- **发现 B（中）**：`append`/`append_consumed` 先落日志再写索引，索引失败即返回 Err → 引擎不 apply、不推进 `last_seq`，日志/内存瞬态分歧直至重启。**修复**：确立**提交点 = JSONL fsync**；索引投影改为 `index_project`（尽力而为，失败不回滚、不失败本次调用），`append_consumed` 并入统一 `append`。同一原则推广到提交点之后的所有派生步骤：`post_commit` 改为**不可失败**（快照失败跳过并保留计数、Terminated 追加失败不回滚已提交命令）。
- **新增决策 D23**：消耗提交契约与 Rule-E 幂等（提交点 / 事件自足 / 幂等 / 原子域含 gate 读集 / 物化边界五点），显式拒绝库事务、2PC、分布式共识。
- **代码变更**：pact-store（`event.rs`/`index.rs`/`store.rs`/`error.rs`）、pact-engine（`EngineError::IdempotencyConflict`、`Command::Consume` 加 `request_id`、会话内幂等表 `consumed_requests`、recover/load 重建、`post_commit` 去 `Result`）、三接口（dto 新增 `ConsumeReq`、server 路由与错误映射、client、mcp `ConsumeIn`、cli `--request-id`）。
- **测试**：store 单元 18（新增 Consumed 自足（空 Counter 上重放）、幂等键序列化，替换 2 项旧用例）；engine 集成 11（新增 request_id 去重 + 跨重启生效 + 同键异 token 冲突）；daemon e2e 补幂等重试断言；`daemon_e2e`/`engine_integration` 全部 consume 调用补 `request_id` 形参。workspace **105 项全绿**；fmt/clippy 清零。
- **文档**：design.md（D23、§6.3 consume 请求、§7.1/§7.3/§7.4、§8.1、新增 §8.5、§9.2/§9.3、§10/§11 计数）；requirements.md（FR9 事件自足/提交契约、FR10 外部副作用边界、FR12 request_id、FR13、FR14 五份域、**新增 FR15**、NFR2、§6 范围外新增一条）；manual.md（§4.2 `--request-id`、新增 §4.5/§4.6、§7.2 Consumed 字段、§9.4 与 §10 增 `IdempotencyConflict`）；architecture.md（§2.1 提交点标注、§5 质量属性）。
- **2a/2b 落盘**：`examples/domains/external_materialization.pact`（witness-token 物化确认示例）+ 手册 §4.6；离线脚本新增 step 1.5、重录 trace（187→200 行，仍为 5 个顶层步骤）、examples/README 与根 README 域数由四改五；`examples/offline/fixtures/se_lifecycle.events.jsonl` 的 5 条 `Consumed` 行补为 token 全量快照以匹配新事件形状（离线 `replay` 因此自洽，已实测通过）。
- **磁盘事件格式变更（无旧格式兼容层）**：`Consumed` 由 `token_id` 改携 `token` 全量对象，属**不向后兼容**的日志格式变更——早于本轮的 `events.jsonl` 在重启重放时会被判为非法行（fail-fast）。本轮为**未发布**的 v1 早期，无历史部署需要迁移，故不加兼容垫片；既有示例 fixture 已重录为新格式。
- **规范母本未触碰**：`docs/pact*` 全程未改；D23 为**实现层**契约（论文仅给单步 SOS 语义），不涉及修订母本。
- **执行偏差**：沙箱缺 `cc` 与 glibc 开发文件，测试链接改用宿主机编译器 + `--sysroot=/run/host`（RUSTFLAGS），命令语义与 CI 一致。
- **安全审视**：仅事件字段、存储写序、可选幂等键与文档/示例；无网络、无 sudo、无密钥、无 unsafe、无新依赖变更；幂等键为客户端自报字符串，仅作 map 键与日志字段，不参与文件路径构造，无新增攻击面。

## 2026-10-15（第 14 轮，通用示例与用户手册落盘）

- **触发**：用户要求补一套与两技能无关的通用结构 + examples，向用户说明 PACT 范式系统的使用场景与方法（需求侧登记为 FR14，见 `docs/requirements-change.md` 第 6 轮）。
- **新增决策 D22**：通用示例与用户手册作为交付资产——不接入 CI、不改任何 crate、运行产物入 `.gitignore`；取证中发现的三处实现与文档不一致只登记不修码。
- **落盘资产**：
  - `examples/domains/`：`se_lifecycle.pact`(45) / `re_pipeline.pact`(50) / `react_loop.pact`(38) / `cross_harness.pact`(36)
  - `examples/fragments/`：`security_review.pact`（成功合并）/ `security_blocking.pact`（C2 冲突）
  - `examples/offline/run.sh`(168) + `trace.txt`(187)：analyze × 4、compose × 2、replay × 1，含三类不良构域与陷阱域
  - `examples/online/run.sh`(251) + `trace.txt`(238)：daemon 启动 → 装载 → 六规则全流程 → Rule-T → 卸载 → 清理（`trap ... EXIT`，因 CLI 无 `daemon stop`）
  - `examples/README.md`(413)：场景导航与逐例讲解
  - `docs/manual.md`(653)：12 章权威用户手册
- **代码零改动**：本轮未修改任何 crate；`cargo test` 结论沿用第 12/13 轮（workspace 103 项全绿）。`.gitignore` 已含 `/examples/online/run-data`、`/examples/offline/out`（脚本每次运行重建）。
- **architecture.md 无变更**：未引入新组件、未改变 crate 划分、进程与部署视图、数据流与质量属性落点，故不追加 architecture-change 记录。
- **规范母本未触碰**：本轮全程未修改 `docs/pact*`（遵守第 13 轮纪律沉淀）；示例与手册中凡引用论文处均以"§x.y → pactv1_rev2.md"形式指向，不复制原文。
- **安全审视结论**：本轮仅新增 Markdown 文档与 shell 教学脚本。脚本无网络出访、无 sudo、无密钥/令牌硬编码（在线示例走信任声明模式，`pact.toml` 中 `[auth.tokens]` 为空表）、无 `unsafe`、无依赖变更；`trap` 清理确保 daemon 与 unix socket 不残留。无新增攻击面，无需调整既有安全控制（恒时令牌比较、事件日志只追加、快照原子写）。

## 2026-10-15（第 13 轮，论文改动回退：规范母本不动，实现登记为保守超集）

- **触发**：用户指出第 12 轮"规范同步（pactv1_rev2.md 三处）"超出实现方工作范畴——论文是规范母本，实现方不得顺手修订。裁决：恢复原文。
- **回退**：§1.4.3 Advisory 段、Appendix A.5 伪代码、A.5 Remediation 三处逐字恢复为参与者级原文（该文件不入仓库，纯本地还原，无版本控制影响）。
- **D21 措辞修正**：删去"与论文一并对齐定案"，改为"论文维持参与者级原文，实现登记为参考算法的**保守超集**；规范是否修订由作者另行裁决"。
- **保留**：`lib.rs` 注释/Display 的类型级表述仍然准确（回退后论文为参与者级、实现更宽，恰构成保守超集关系）；跨参与者回归用例；第 12 轮全部测试结论（103 项全绿，本轮代码零改动）。
- **纪律沉淀**：涉及规范母本（`docs/pact*`）的任何文字改动，事前必须单列请求用户显式确认，不得随"更新文档"类概括指令默认授权；"实现宽于规范"的偏差以保守超集登记于 design.md，不回灌论文。

## 2026-10-15（第 12 轮，SeedProduceOverlap 语义注释与规范对齐定案）

- **背景**：技能适配轮发现 `precondition_wf` 的 `SeedProduceOverlap` 实现**宽于**其文档注释——`produced` 收集为全体参与者 produces gate 的全局类型集，跨参与者组合（SE `responsible_human` seeds + RE `release_manager` produces）也告警，方向为假阳性不漏报；既有测试只覆盖同参与者场景（skill-adaptation.md §10 第 1 轮登记）。用户裁决采处置 (b)：改注释承认现状、保留实现。
- **代码变更（逻辑零变更）**：pact-analysis `WfWarning::SeedProduceOverlap` 文档注释改为**类型级判定**表述（produces gate 取全体参与者并集，`participant` 字段 = seed 持有者；比论文 §1.4.3 原表述更宽，假阳性方向不漏报）；`Display` 文本同步改写（旧文"同时持有 seed 权威与 produces gate"暗示同一参与者，与实现不符）。无测试断言依赖旧 Display 文本，JSON 序列化形态不变（变体名 + 字段）。
- **规范同步（pactv1_rev2.md 三处）**：§1.4.3 Advisory 段改为类型级定义——重叠 = 非源类型 T 同时可 seed 与可 produce，同参与者双入口是退化情形，跨参与者重叠亦须显式部署裁决（recall 偏向，不漏报同参与者情形），警告 `participant` 字段为 seed 持有者；Appendix A.5 伪代码改为 `Produced ← { T : ∃ p, ("produce", T) ∈ Gate(p) }` 后按类型判属；A.5 Remediation 补类型级说明。
- **测试**：`precondition_wf_warns_seed_produce_overlap` 增补第三块——跨参与者用例（p produces b、q seeds b、q ≠ p ⇒ 警告 `participant = q`），锁定类型级行为（第 1 轮偏差今后由测试捕获）。workspace 全量 **103 项全绿**（净增 0 项，扩充既有用例）；fmt/clippy 清零。
- **D21 同步**：design.md D21(4) 补记类型级判定定案（produced 并集、participant 字段语义、假阳性方向）。
- **执行偏差（沿用）**：distrobox `pact-dev` 仍不可达，验证在宿主机工具链（cargo 1.98.1，与 CI stable 同版）完成，命令与 CI 一致（fmt --check → clippy --locked -D warnings → test --workspace --locked）。
- **安全审视**：本轮仅注释、Display 字符串、测试断言与文档，无网络、无进程、无密钥、无 unsafe、无依赖变更（`--locked` 通过即证 Cargo.lock 未动）；不触发推送/部署前置审视的重新评估，2026-09-06 第 8 轮结论继续有效。

## 2026-09-09（第 11 轮，CI 偶发失败修正：场景 B Counter 卫生断言）

- **现象**：GitHub Actions 上 `unattended_e2e::unattended_scenarios_to_terminated` 偶发失败——`max_counter = 3` 违反"应恒 ≤ 2"断言；同轮本地（20+ 连跑）与全量回归均未复现。
- **根因（非引擎缺陷，为测试断言过强）**：场景 B 的 P1 纪律中，`pact_produce` 与 consume-after-produce 的 `pact_consume` 是同一 agent 内的两次串行调用，窗口内 Counter = 2；若下一阶段 agent（被 produces 推送或轮询唤醒）在该窗口内完成自己的 Rule-P，Counter 合法地达到 3（理论链式重叠上界为类型数 5）。Rule-P 只要求前驱在 Counter 上（不被消费），且 §3.1 规定 `produces` 门不接受条件子句——该窗口**语义上不可消除**，窗口重叠是 PACT 自治语义下的合法执行。本地 20 连跑通过纯属调度时序有利。
- **修正**：场景 B 卫生断言由 `≤ 2` 放宽为结构上界 `≤ 类型数 5`（每类型至多一实例：每类型仅一个 produce 门持有者 + P1 同迭代串行 produce→consume），注释记录竞态分析与 CI 实测证据。场景 A（单 agent 串行，无并发生产者）的 `≤ 2` 为真不变量，保留不动。
- **验证**：unattended_e2e 本地 10 连跑稳定；workspace 全量（104 项）+ fmt/clippy 清零。
- **顺带**：design.md §11 阶段 5 验收表述同步修正（场景 B 卫生上界）。

## 2026-09-09（第 10 轮，论文 rev2 对齐重构）

- **论文切换**：范式规范源由 `pactv1_rev1.md` 切换为 `pactv1_rev2.md`（design.md 头部引用更新）。rev2 实质差异六项：§1.4.4 良构约束扩展（新增 LHS Uniqueness、Non-Empty LHS；自环合法）、Rule-P Note（前驱互异只按 TokenId）、§1.4.3 Advisory（Seed/Produce 重叠为警告非违规）、§1.7.3 PosNat（count 字面量 `[1-9][0-9]*`）、§3.1 六项良构约束重述、Appendix A.5 重写（三元输出 + 附录 D 前提顺序）。
- **D21 新增（用户选定三项推荐方案）**：良构约束强制风格与 A.5 重构——空 LHS 解析期拒绝 + `EmptyPrecondLhs` 声明期防御；LHS 重复在 `build_domain` 原始序列上检测（BTreeSet 表示层静默去重，唯一强制点）；`pos_nat()` 拒绝 0/前导零 + `ZeroCountLiteral` 防御；A.5 = `precondition_wf` 三元报告（委托 `validate_domain` 单一判定源 + `SeedProduceOverlap` 咨询性警告）；`AnalysisReport.precondition_wf` 字段前置。
- **代码变更**：pact-core 新增 3 个 `DomainError` 变体（`DuplicatePrecondLhs` / `EmptyPrecondLhs` / `ZeroCountLiteral`）+ `validate_domain` 空 LHS 与零计数检查 + `GateExpr::zero_count_refs`；parser `nat()` → `pos_nat()`、花括号形式空 LHS 语法拒绝；pact-analysis 删除 `SourceTypeProduction` / `source_production_legality`，新增 `WfWarning` / `PreconditionWfReport` / `precondition_wf`；engine `AnalysisReport` 字段重命名前置；cli 摘要输出改造（`precondition_wf: 良构（n 项违规，m 项警告）` + 警告逐条）；rules.rs 按 Rule-P Note 更新注释（逻辑零变更，`provided.len() != predecessors.len()` 非冗余——排除同类型多实例冒充）。
- **表示层决策**：`DomainError::UndeclaredType.referenced_by` 由 `&'static str` 改 `String`——`&'static str` 使含该错误的容器无法 derive `Deserialize`（serde 生命周期约束），A.5 报告需内嵌 `Vec<DomainError>` 故必须自有。序列化形态不变（同为 JSON 字符串）。
- **测试**：新增 rev2 专项（parser：`{A+A}→B` 拒绝、空 LHS 拒绝、count 0 / 前导零拒绝、多位 PosNat 接受、自环接受；analysis：`precondition_wf` 良构/违规/警告三元结构、`EmptyPrecondLhs` / `ZeroCountLiteral`（含 Not 嵌套）防御、`SeedProduceOverlap` 正负用例）；更新既有用例（`count b = 0` → `count b = 2`；三接口 e2e 的 analyze 断言改为三元结构）。workspace 97 → **103 项测试全绿**；fmt/clippy 清零。
- **无架构变更**：crate 结构、依赖方向、数据流、部署拓扑不变（architecture.md 不动）。

## 2026-09-07（第 9 轮，CI 接入 + lint 缺口归零）

- **新增 GitHub Actions**（`.github/workflows/ci.yml`）：push(main)/PR/workflow_dispatch 触发；单 job 依次执行 `cargo fmt --all -- --check` → `cargo clippy --workspace --all-targets --locked -- -D warnings` → `cargo test --workspace --locked`；dtolnay/rust-toolchain@stable（含 rustfmt、clippy）+ Swatinem/rust-cache。无设计变更。
- **历史缺口归零**：rustfmt/clippy 不可用问题已解决——沙箱为独立 rootfs（Fedora，无 rustup），经官方 rustup 装入 `~/.local/share/rustup`（stable 1.98 + rustfmt 1.9 + clippy 0.1.98）。
- **全仓 lint 规范化**：①`cargo fmt --all` 一次性基线（rustfmt 2024 风格，纯机械重排）；②clippy 1.98 全量 36 处告警清零——31 处机械修复（`is_none_or`、`keys()` 迭代、`io::Error::other`、`from_ref` 代 clone、未用导入、布尔比较等），5 处手工（doc 列表缩进 ×3 文件、`ClausesByParticipant` 类型别名、测试模块更名 `mod unit` 消 inception、`&PathBuf`→`&Path`、cli_e2e 失败路径 kill+wait 防僵尸进程）。
- 回归：stable 1.98 工具链下 fmt --check ✓ / clippy -D warnings ✓ / **97 项测试全绿**。

## 2026-09-06（第 8 轮，推送前全面安全审视）

- **范围**：整个 workspace（9 crate + docs + 仓库文件面），AGENTS.md 推送/部署前强制约束。方法：项目安全基线 → 偏差图 → 源到汇三遍过；横切扫描 `unsafe`/进程/环境变量/密钥样式串/SQL 拼接/路径拼接。
- **结论**：路径穿越（域名 `[A-Za-z0-9_-]{1,128}` 强校验）、SQL 注入（turso 全参数化）、不安全反序列化（仅 serde 强类型）、命令注入（生产代码零进程 spawn）、密钥泄露（仓库零命中、bearer 不落日志、`.gitignore` 覆盖 target/data/sock）均清洁；core/parser/analysis/compose 保持零 IO 纯度。
- **唯一 LOW 级发现（已修复，用户选定"现在修复"）**：[CWE-208] `ServerState::authorize` 的 bearer token 比较为非恒时的 `String ==`（TCP 模式下可计时侧信道）。修复：pact-http `server.rs` 新增零依赖恒时比较 `ct_eq`（长度差计入 diff + 字节异或折叠），并附单元测试锁定语义。**无设计变更**（§9.4"接口层校验"语义不变）。
- **设计内部署注意（记录，非变更）**：读/管理端点不校验 bearer 为 D11 信任模型既定（TCP 模式下本机进程可读状态/装卸域，v1 默认 unix socket 为边界）；`pact.toml [auth] tokens` 明文，部署建议 chmod 600；`--bearer` 经命令行本机 ps 可见，v2 换签名密钥；client URL 拼接未编码但影响仅 404/405；turso BETA 由 D13 隔离层对冲。
- 回归：workspace 累计 **97 项测试全绿**（新增 1 项 `ct_eq` 单元测试）。

## 2026-09-05（第 7 轮，阶段 5：无人值守场景验证）

- **无设计变更**：阶段 5 为纯验证阶段，未改动引擎/核心语义。新增 `pact-mcp/tests/unattended_e2e.rs`（两场景合并为单 `#[tokio::test]` 顺序执行），落实 FR11 与 §11 阶段 5 验收——测试进程只做"外部启动机制"（拉起 daemon + N 个真实 `pact-mcp` stdio 子进程），此后零 Rule-S/P/R/E 介入，agent 经 P1/P2/P3 策略 + Rule-W（订阅推送 + 轮询兜底）自驱至 Rule-T。
- **验证发现（印证既有设计，非变更）**：域加载与种子注入之间存在窗口，此时 Counter=∅ 且无 token，`termination_holds` 平凡为真、`pact_status` 返回 `terminated=true`。这与 §"Terminated 为信息性事件、不锁定域（种子仍可注入）"一致，属**预期语义**；无人值守 agent 驱动层须自行防护——仅当 `archive_size>0`（确有真实工作发生）才承认终止，避免 worker 在播种前误判退出。修复落在测试的 agent 驱动器，引擎零改动。
- **agent 驱动层竞态修复（测试内）**：①上述过早终止防护；②Terminated 一次性推送与轮询判定竞态——轮询可能先于推送判定终止，故终止后开 500ms 宽限窗口排空在途推送，确保 `notified_terminated` 不漏计；③P2 汇消费收紧为"仅当无 produces 门或生产预算耗尽"才允许，防止有生产门的 agent 误食自己的见证前驱破坏 consume-after-produce 纪律。
- **测试基架**：两场景合并单 runtime 顺序执行，规避同进程多 runtime 并发 reap `pact-mcp` 子进程的 tokio 全局孤儿队列陷阱。
- 阶段 5 完成：workspace 累计 96 项测试全绿；unattended_e2e 20/20 连跑稳定。§11 阶段表阶段 5 标记为已完成。

## 2026-09-04（第 6 轮，阶段 4：pact-http + pact-cli + pact-mcp）

- **D19 新增（用户决策）**：pact-http 提前至阶段 4，与 pact-cli、pact-mcp 同批交付；MCP 与 CLI 不内嵌引擎，统一经 `pact_http::client`（REST over unix socket）接入同一 daemon；原规划的 `pact-server` 组装 bin 取消，daemon 由 `pact daemon start` 与 `pactd` 提供。接口等价性由构造保证（三接口只是同一 REST 的形态转换）。
- **D20 新增**：CLI 域操作一律 `--domain/--participant/--token/--type` flag（放弃 §9.3 原稿 `pact enabled <participant>` 位置参数简写）；全局连接参数 `--unix/--tcp/--bearer/--json`；MCP v1 仅 stdio，Streamable HTTP 移交联网演进（v2）。
- §1 crate 清单更新：删 pact-server 行；pact-http 注明含类型化 client 与 pactd bin；依赖方向改为 `http → engine`、`mcp/cli → http(client)`、`cli → parser/analysis/compose/core`（离线命令）。
- §9.1/§9.2/§9.3 按实现回填：MCP 经 REST 转发；`{d}:analyze/:compose` 段内冒号在 `/domains/{d}` 统一入口分发（matchit 限制，对外 URL 不变）；独立 `POST /validate`；CLI 完整命令/flag 清单逐条对齐实现。
- §11 阶段表：阶段 4 完成（95 项测试全绿：新增 HTTP daemon e2e、CLI e2e×2、MCP stdio e2e）；阶段 5 范围调整为纯无人值守场景验证（pact-http 已提前交付）。

## 2026-08-26（第 5 轮，阶段 3：pact-store + pact-engine）

- **store schema 修正**（§7.3 落地偏差，代码注释已先行标注）：
  - `events` 表主键由 `seq` 改为 `(domain, seq)`——seq 是域内单调序，跨域必然碰撞；
  - `DomainLoaded` 事件携带 `seq`/`ts`——使"日志逐行 +1 单调"不变量对所有行统一（含域加载行）。
- **engine 命令全集落定**（§8.1 枚举补全）：新增 `ListDomains`、`Shutdown`（全部域终态快照后退出）；`Compose` 定名 `ComposeAndLoad`（组合 + emit 回写 + 加载一体）；`Subscribe` 返回 `broadcast::Receiver<Notification>`。
- **域替换语义明确**：同名额重载 = 重置会话状态 + 清理陈旧快照 + seq 续接；恢复时快照仅在 `snapshot.seq ≥ 最后一次 DomainLoaded.seq` 时有效，且只重放该事件之后的事件（防替换后旧状态复活）。
- **Terminated 事件一次性**：仅在 `termination_holds` false→true 转移时追加并广播，重复求值不重复落盘。
- **恢复 fail-fast**：日志中部损坏（完整行非法 JSON）或声明不可解析时，actor 以统一 `EngineError::Recovery` 排空命令队列（需人工介入）；尾部半行照 §7.4 截断自愈。
- **Clock 无 chrono 依赖**：`SystemClock` 以 civil-from-days 手算实现 ISO-8601 UTC（修复实现中月份变量遮蔽分钟的 bug，测试锁定闰日纪元）。
- 阶段 3 完成：31 项新增测试全绿（store 17 / engine 单元 4 / engine 集成 10），workspace 累计 80 项；纯度核验通过（pact-core 依赖树仅 serde）。

## 2026-08-26（第 4 轮，论文 rev1 对齐 + 阶段 2）

- **论文切换**：范式规范源由 `pactvfinal.md` 切换为 `pactv1_rev1.md`（design.md 头部引用更新）。
- **D15 回滚**：论文 rev1 以源生产约束（Rule-P 前提 `Precond(T) ≠ ∅` + 声明期 WF-3 拒绝）原生解决 Rule-T 终止问题，临时豁免（termination_holds 排除空前置组合）失去存在意义，代码与文档全部回滚。
- **D17 新增**：源类型生产约束三层强制——(1) `rule_p` 运行时前提检查（`RuleError::SourceTypeProductionDenied`）；(2) `validate_domain` 声明期拒绝（`DomainError::SourceTypeProduceGate`，WF-3）；(3) `is_source_type` 查询函数 + A.5 静态分析独立复核。`termination_holds` 按 rev1 Rule-P-enabled 三条件重写；`enabled_productions` 不返回源类型。
- **D18 新增**：compose C2 实践语义——共享类型前置一致性判定为"双侧都声明规则且集合不同才冲突，单侧声明合法"（论文 §1.8 示例 D₂ 共享 design 而不重复声明其前置的规范语义）。
- 影响范围与同步更新：LIFECYCLE 黄金域改为 rev1 版（analyst 由 produce gate 改为 seed authority、operator via 子句移除）；pact-core 单元测试补源类型约束专项；property 宽容域去掉源类型 produce gate；parser 测试补 WF-3 用例。
- 阶段 2 完成：pact-analysis（六项分析 A.1–A.6，petgraph SCC/拓扑）+ pact-compose（四条件组合）落地，20 项测试全绿。
- 阶段 1 状态更新为"2026-08-26 按 rev1 更新"（27 项测试全绿）。

## 2026-08-25（第 3 轮，阶段 1 实现）

- **D15 新增**：Rule-T 终止判定的"enabled"排除空前置组合（源类型生产）——字面语义下含源类型 produce gate 的域永不终止，与论文 Part II 轨迹 t₁₀/t₁₃ 矛盾；排除后判定在 Counter=∅ 前提下等价于 Counter 为空。黄金测试 + 专项单元测试（`termination_semantics`）锁定。
- **D16 新增**：parser 宽容度决策（段序严格/尾分号与空段宽容/`->`、`//` 注释、GateExpr 优先级 not > and > or 为扩展）。
- 实现落定与接口微调：
  - `GuardedAction` 新类型（Read|Consume）类型层面强制 GateHolds 作用域（§1.4.0）；
  - `Token.ty` 线上序列化字段名 rename 为 `"type"`（§6.2 契约，测试锁定）；
  - `Effect::token_id()` 辅助方法（引擎广播/持久化用）；
  - `GateExpr::type_refs()`、`precond_of()` 辅助查询（良构校验/分析用）；
  - pact-core dev-dependencies 增补 serde_json（仅测试，验证线上 JSON 契约；不违反纯度白名单——白名单约束 runtime 依赖）。
- 阶段 1 状态：已完成（26 项测试全绿；纯度核验通过）。见 design.md §11。
- 环境缺口：pact-dev 容器无 rustup/sudo，rustfmt 与 clippy 不可用，待补齐后补跑。

## 2026-08-25（第 2 轮）

- **D13 新增**：派生索引引擎采用 turso（原 Limbo，Rust 重写的 SQLite，现 BETA）。可行性依据：索引在本架构中可随时从 JSONL 真相源重建，beta 风险被对冲；收益为纯 Rust 工具链与 async API。经 pact-store 内部 `IndexStore` trait 隔离，保留 rusqlite 后备退路。存储布局中 `index.sqlite` 改为 `index.tursodb`。
- **D14 新增（pact-core 纯度审计结论）**：
  - Rule-T 终止判定（`termination_holds`）、Rule-W 可满足性判定（`enabled_productions`）、域良构校验（`validate_domain`）从引擎/parser 层移入 pact-core 纯函数——语义收口，引擎只编排不判定；
  - 新增 `Effect` 枚举与 `CounterState::apply`，状态变更路径唯一；
  - 明确纯度硬性清单：无 IO / 无日志（tracing 宏禁用）/ 无时钟 / 无随机 / 无全局可变状态 / 依赖白名单（serde 仅 derive）；
  - `rule_s` 签名去掉 `state` 参数（论文 Rule-S 前提不含 Counter 条件）。
- 新增 §6 领域数据格式：`.pact` 声明文件示例、Token/ArchiveEntry JSON、动作请求/响应与结构化错误格式、通知事件格式、查询响应格式（三接口共享）。
- §2、§3 扩充为阶段 1 两模块的完整接口定义（类型 + 函数签名）。
- §10 测试策略改为四层组织（单元/属性/集成/端到端+回归），对齐 FR13。
- 文档标题去掉"（当前有效）"后缀。

## 2026-08-25（第 1 轮）

- 初始设计建档（D1–D12，crate 划分，核心类型，MCP 工具清单，REST 端点，测试策略，六阶段计划）。
