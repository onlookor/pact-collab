# TODO（进度台账）

> 纪律见 `AGENTS.md`：每轮开始先读本文件确认事项与状态；每轮结束更新状态、验证证据与提交哈希。
> 状态：`[ ]` 待做 · `[~]` 进行中 · `[!]` 阻塞 · `[x]` 完成。完成项移入末尾“已完成归档区”。
> 依据列只引用权威文档编号；新编号先登记到 `docs/architecture.md` 附录 A 再使用。
> 修复顺序以本文件为准；调整、降级或拆分必须写明原因。

当前状态（2026-10-31，第 90 轮）：P0 入口校验（REST + WASM，D50）、P1 的 `pinning` / `onUnknownScheme` / `value`、B0 卡片加载（FR32/D47）、B1 显式重载（FR33/D48）、`declarationDigest` v1 两步（FR34/D49）与物引治理 SKILL 均已落地并回归通过。`refd` 的端点契约与鉴权已于本轮定案（治理 §13.6、`spec/ref/v1/reference-plane.openapi.yaml`），实现待做；动态阶段未开始。下一轮按 P3 实现 `refd` 的 `/v1/resolve` 与 `/v1/describe`（含身份/ACL/审计）。

## 当前批次：物引（ThingRef）治理收尾（2026-10-28 起）

建议顺序与原因（来自第 76 轮审视）：

1. **先定入口边界**——校验放 REST 写入边界，引擎保持不透明；CLI/MCP/runtime 经 REST 自动继承，
   避免各入口各写一套。
2. **修确定性缺陷**——`git+ssh` 误拒、Identifier 文法未执行，两处与 `spec/ref/v1/uri-profile.md` 不一致。
3. **缺失/`null` 区分与绑定策略**——把规范层已有的 `RefBindings` 规则接到写入路径。
4. **归档钉版与运行期策略**——`pinning` / `onUnknownScheme` / `value` / `declarationDigest`。
5. **跨仓库快照与治理平面**——`pact-ref` vendored 同步、`refd` 端点、动态阶段。

### P0 入口票面校验（FR30 / D45）

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [x] | 边界定案：校验在 REST 写入边界（`pact-http`），引擎与 `pact-core` 不依赖 `pact-ref-spec`；CLI/MCP/runtime 经 REST 继承 | FR30 / D45 | `docs/design.md` §9.12 记录；`pact-engine` 依赖树无 `pact-ref-spec` |
| [x] | 三态：字段缺失 → `400`、显式 `null` → `∅`、字符串 → 有引用（`ThingRefField`） | FR30 / `uri-profile.md` §1 | `pact-http` 单测 `thing_ref_field_distinguishes_missing_null_and_value`；e2e `entry_validation_structural_rules` |
| [x] | URI profile 结构规则：控制字符、裸路径、密码型 userinfo、长度 | FR30 / `uri-profile.md` §2–§3 | `pact-ref-spec` 单测 + e2e（`InvalidThingRef`） |
| [x] | Identifier 文法 `1*(ALPHA/DIGIT/_/-/./:/@/#)` | FR30 / `uri-profile.md` §3 | `pact-ref-spec` 单测 `rejects_identifier_characters_outside_grammar` |
| [x] | `git+ssh://git@host/...` 不再误判为密码型 userinfo | FR30 / `uri-profile.md` §3 | 单测 `allows_ssh_username_but_rejects_password_userinfo` |
| [x] | RefBindings 策略：`required` / `allowIdentifier` / `schemes` / `maxLength` | FR30 / D45 | e2e `entry_validation_applies_bindings_policy` |
| [x] | 错误码映射：`ThingRefMissing` / `ThingRefRequired` / `InvalidThingRef`，均为 `400` | FR30 / D45 | `server.rs::validate_entry_thing_ref` + e2e 断言 |
| [x] | 大写入上限：结构校验硬上限 `HARD_MAX_REFERENCE_LENGTH = 65536`，绑定 `maxLength` 不得超此值 | FR30 / `uri-profile.md` §2 | `pact-ref-spec` 常量与 `validate_rule` 测试 |
| [x] | MCP `pact_seed` / `pact_produce` 的缺失语义：工具参数省略由 client 发显式 `null`（= `∅`），无法表达“缺失” | FR30 / D45 | 已定案并写入 `manual.md` §7.11：MCP 省略 = `∅`；如需严格缺失另立事项 |
| [x] | `pact-wasm` 本地入口校验：`host.rs` 直接构造 `ThingRef`，未做 URI profile / 绑定校验 | FR30 / D50 / `thingref-governance.md` §8.2 | 入口语义抽为 `pact-ref-spec::validate_entry` 纯函数（REST 委托、WASM 复用）；`pact-wasm` 单测 3 项（绑定注入与规范化、produce 校验与钉版、非法绑定/移除/清空）；Node 测试 `refBindings 注入后在写入前校验并规范化票面`；wasm32 clippy 与 `ci-smoke` 通过；全量 373 passed |
| [x] | `pact-runtime` 种子/生产路径确认继承 REST 校验 | FR30 | `pact-runtime` 经 `RestClient` 调 REST；`runtime_e2e` 5 passed（含旧 Identifier 域需 `allowIdentifier: true` 的回归修正） |

### P1 归档钉版与运行期绑定策略

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [x] | `pinning` 运行期强制：`required` 在写入时、`required-at-archive` 在 consume 进入 Archive 前要求钉版摘要（FR31/D46） | FR31 / D46 / `uri-profile.md` §4 | 单测 + e2e：写入/consume 缺摘要 → `400 ThingRefUnpinned`；钉版可归档 |
| [x] | `onUnknownScheme` 运行期生效（`reject` / `treat-as-identifier` / `allow`） | FR31 / D46 / `uri-profile.md` | 单测三分支 + `treat-as-identifier` 未放行 Identifier 被拒 |
| [x] | `RefRule.value`（`none`/`uri`/`identifier`）语义落地 | FR31 / D46 / `ref-bindings.schema.json` | 入口强校验 + 单测；`required=true` 与 `value=none` 由绑定校验拒绝 |
| [x] | B0：ContractCard `artifact.digestPolicy` / `reference.pinning.requiredForArchive` 运行期加载；daemon 新增 `[ref_contracts].paths`，按 `contract_id()` 建索引；声明 `contract` 的部署缺卡即拒绝启动；重复 `id@version` 即拒绝启动 | FR32 / D47 | 单测：加载/索引、按数据触发、重复 `id@version`、钉版并集只加严；e2e：卡片钉版阻断未钉版 consume（`ThingRefUnpinned`）、重复版本拒绝启动；`pact-http` 全绿，全量 338 passed；提交 `4b42778` |
| [x] | B1：显式重载 `POST /ref/reload` + `pact ref reload`；卡片与绑定同事务、原子换索引、`dryRun` 影响报告、`ref-reload.jsonl` 审计、ACL 覆盖绑定域；不做文件监听 | FR33 / D48 | 单测 3 项（审计 JSONL、审计失败、并发 `409 RefReloadBusy`）；`ref_reload_e2e` 7 项（`dryRun` 报告、成功重载后新规则生效、失败保持旧状态、ACL 当前/新域、认证/信任模式、未配置 500）；CLI e2e 1 项；全量 349 passed；`fmt`/`clippy` 通过；提交 `a514380` |
| [x] | `declarationDigest` v1 第一步：语义接口摘要（域名 + types + preconditions + gates 含条件 + seed）、紧凑 JSON 规范化与条件文法冻结；`pact ref digest` 工具 + lint 漂移比对（不一致 error） | FR34 / D49 / `spec/ref/v1/declaration-digest.md` | `pact-analysis` 单测 5 项（含 golden 向量）；`pact-cli` 单测缺失/不一致；`ref_lint_e2e` 真实二进制 `ref digest`、`--name`、非法域名、漂移退出 1；全量回归见验证表 |
| [x] | `declarationDigest` 第二步：运行期比对 `[ref_digest] mode = off|warn|enforce`（默认 `warn`），仅在 `POST /domains`（`load_domain`）比较；`enforce` 仅“摘要存在且不一致”返回 `400 DeclarationDigestMismatch`；启动恢复与 D48 重载不比较 | FR34 / D49 第二步 / 治理 §13.4.2.1 | `pact-http` 单测 `digest_mismatch_modes_and_boundaries`；`ref_digest_e2e` 5 项；全量 363 passed；`fmt`/`clippy` 通过 |

### P2 跨仓库与快照

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [x] | `pact-ref` vendored 快照同步（第 77 轮：`HARD_MAX`、Identifier 文法、userinfo、`InvalidIdentifier`；第 78 轮：`merged_rule`、`pinned_digest`、`classify_identifier`、新错误；第 88 轮：`validate_entry` / `EntryRef` / `EntryError`） | `AGENTS.md` 跨仓库纪律 | 第 88 轮 `scripts/sync-spec.sh --source … --check` 通过（`pact-ref` 工作区仅 vendored 快照变化） |
| [x] | `pact-ref` 回归：vendored 快照更新后跑 `cargo fmt/clippy/test` | 同上 | 第 88 轮 **57 passed / 0 failed**（`pact-ref-spec` 40，含新增 7 项入口测试），`fmt`/`clippy` 通过；`pact-ref` 提交 `9be7cbd` |
| [!] | `pact-ref` conformance：补入口校验用例（缺失/`null`、userinfo、Identifier 文法、`maxLength`） | `conformance/` | 阻塞：`conformance/runner` 仍是骨架（不执行用例）；待 runner 落地后补 |

### P3 治理平面与动态阶段

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [x] | `refd` resolve/describe 端点契约与鉴权定案（路径、请求/响应、错误模型、身份/ACL/审计、版本） | `thingref-governance.md` §13.6 / `spec/ref/v1/reference-plane.openapi.yaml` | 治理 §13.6 十小节定案；OpenAPI 重写为 `/healthz`、`/readyz`、`/v1/resolve`、`/v1/describe`；`yaml.safe_load` 与全部 `$ref` 校验通过 |
| [ ] | `refd` 实现 `/v1/resolve` 与 `/v1/describe`（身份、scope + ACL、审计 fail-closed、file/https adapter） | 同上 §13.6.9 实现映射 | 端点可按 OpenAPI 契约调用；401/403/400/409/500 分支有测试；审计记录用例；跨仓库 `pact-ref` |
| [ ] | 动态只读阶段（`mcp-read` profile、resolve 适配器、物化结果） | `thingref-governance.md` §13.1 阶段 4 | 端到端：resolve → witness token |
| [ ] | 动态写阶段（`mcp-mutating` profile、审计边车、witness/compensation） | 同上阶段 5 | 审计边车用例 |
| [ ] | 可选注册表（只做发现，不做真相源） | 同上阶段 6 | 触发条件满足后再开 |

### 文档同步（本轮必须完成）

| 状态 | 事项 | 验收方式 |
|---|---|---|
| [x] | `docs/requirements.md` 新增 FR30；`requirements-change.md` 第 38 轮 | 与代码行为逐条对齐，含错误码 |
| [x] | `docs/design.md` 新增 D45 与 §9.12；`design-change.md` 第 77 轮 | 记录边界、三态、错误码、绑定策略、MCP/WASM 边界 |
| [x] | `docs/architecture.md` 正文补“REST 边界校验，引擎不透明”，`architecture-change.md` 第 38 轮 | 附录 A 已登记 `FR30`/`D45` |
| [x] | `docs/manual.md`：REST seed/produce 的 `thing_ref` 三态、错误码、`ref_bindings` 策略 | 与 e2e 实测一致 |
| [x] | `docs/thingref-governance.md` §8.1 / §13.1：入口校验状态改为已实现（REST 边界），并标注 MCP/WASM 边界 | 与 FR30 引用一致 |
| [x] | 第 78 轮：`docs/requirements.md` FR31 + 第 39 轮；`docs/design.md` D46 与 §9.13 + 第 78 轮；`architecture.md` 附录 A/正文 + 第 39 轮；`manual.md` §7.11；治理 §8.1/§13.1 | 与代码及 e2e 实测一致 |
| [x] | 第 80 轮：治理 §13.4.1 选项 C 改为“按数据触发闭环”，新增 §13.4.3“摘要误算是否阻塞域加载”；`design-change.md` 第 80 轮；无代码变更 | 与 §13.4 方案一致；仅文档，无需回归；提交 `0115315` |
| [x] | 第 81 轮：治理新增 §13.5“ContractCard 生命周期：重启、显式重载与版本化”，并标注 §13.4.1 第 4 条为首版激活机制；`design-change.md` 第 81 轮；无代码变更 | 与 D27/D39 及卡片 `id@version` 语义逐条对照；仅文档，无需回归；提交 `246d4a8` |
| [x] | 第 82 轮：登记 `FR32`/`D47`（B0）与 `FR33`/`D48`（B1）；`requirements.md` 新增两节、`requirements-change.md` 第 40 轮；`design.md` §0 新增 D47/D48；治理 §13.4.1/§13.5 标注已确认与编号；`architecture.md` 附录 A 登记；`architecture-change.md` 第 40 轮 | 编号先登记后引用；仅文档，无需回归；提交 `f98c698` |
| [x] | 第 83 轮：B0 实现同步——`requirements.md` FR32 状态、`design.md` §9.14 与 D47、`manual.md` §2.2/§7.11、治理 §8.1/§13.1/§13.4.1/§13.5、`architecture.md` 正文；`design-change.md` 第 83 轮、`architecture-change.md` 第 41 轮 | 与代码及测试逐条对照；全量 338 passed |
| [x] | 第 84 轮：B1 实现同步——`requirements.md` FR33 状态、`design.md` §9.15 与 D48、`manual.md` §2.2/§7.11/§7.12/§8.1/§8.2/§9.4、治理 §8.1/§13.1/§13.4.1/§13.5、`architecture.md` 正文、`dist/pact.container.toml`；`requirements-change.md` 第 41 轮、`design-change.md` 第 84 轮、`architecture-change.md` 第 42 轮 | 与代码及测试逐条对照；全量 349 passed |
| [x] | 第 85 轮：`declarationDigest` 候选 v1（语义接口、紧凑 JSON 规范化、工具+lint 先行、运行期 `warn`/`enforce`）写入治理 §13.4.2.1；`design-change.md` 第 85 轮 | 仅文档，无代码变更，无需回归；**待用户确认后再登记 FR/D、配置项与错误码** |
| [x] | 第 86 轮：登记 `FR34`/`D49`（`architecture.md` 附录 A）；新增 `spec/ref/v1/declaration-digest.md` 与 schema `description`；`requirements.md` FR34、FR27/FR30 边界、`requirements-change.md` 第 42 轮；`design.md` D49/§9.16/D41/§9.12/§10、`design-change.md` 第 86 轮；`manual.md` §7.8/§7.11/§7.13/§8.1；治理 §8.1/§13.1/§13.4.2/§13.4.2.1；`architecture-change.md` 第 43 轮 | 编号先登记后引用；文档与代码/测试逐条对照；跨仓库标注：无 `pact-ref` 代码改动；提交 `2bffcd8` |
| [x] | 第 87 轮：登记 `[ref_digest]`/`[ref_digest].mode`/`DeclarationDigestMismatch`（`architecture.md` 附录 A）；`requirements.md` FR34 第二步与 FR30、`requirements-change.md` 第 43 轮；`design.md` D49/§9.4/§9.16/§10、`design-change.md` 第 87 轮；`manual.md` §2.2/§7.11/§7.13/§7.14/§8.2/§9.4/§10；治理 §8.1/§13.1/§13.4.2/§13.4.3/§13.4.2.1；`architecture.md` 正文、`architecture-change.md` 第 44 轮；`dist/pact.container.toml` | 配置项与错误码先登记后引用；文档与代码/e2e 逐条对照；跨仓库标注：无 `pact-ref` 代码改动；提交 `505e81d` |
| [x] | 第 88 轮：登记 `D50`（`architecture.md` 附录 A）；`requirements.md` FR21/FR30/FR31、`requirements-change.md` 第 44 轮；`design.md` D50/§9.12/§9.17/§10/§16.4、`design-change.md` 第 88 轮；`manual.md` §2.7/§7.11；治理 §8.1/§8.2/§13.1/§13.2；`architecture.md` 正文、`architecture-change.md` 第 45 轮；README 中英文本地嵌入改为用途视角；`todo.md` 新增 P4 物引治理 SKILL | 编号先登记后引用；文档与代码/测试逐条对照；跨仓库：`pact-ref/vendor/pact-ref-spec` 已同步，`pact-ref` 57 passed |
| [x] | 第 89 轮：新增 `skills/pact-thingref-governance/SKILL.md`（六主题、适用闸门、不变量、闭环验收、输出模板、边界）；`design-change.md` 第 89 轮；纠偏 `spec/ref/v1/declaration-digest.md` §4“运行期比对尚未启用” | 技能不引入新编号（不占用 `S0`–`S7`）；命令与 `docs/manual.md` §7.8–§7.14 逐条对齐；规范文档与 D49 第二步实现一致 |
| [x] | 第 90 轮：治理新增 §13.6“refd 的 resolve/describe 端点与鉴权”（十小节）；§9.2/§13.1 状态改为“契约已定案、实现待做”；重写 `spec/ref/v1/reference-plane.openapi.yaml`；`design-change.md` 第 90 轮；跨仓库同步 `pact-ref/docs/architecture.md` 的 API 段落与引导阶段 | 契约与卡片 schema 逐字段对照；`$ref` 全部可解析；跨仓库改动已在 `design-change.md` 标注 |

### 验证与交付

| 状态 | 事项 | 验收方式 |
|---|---|---|
| [x] | 第 77 轮全量回归 `cargo test --workspace --locked` | **317 passed / 0 failed**（50 个 test binary，含 `runtime_e2e` 5 passed） |
| [x] | 第 78 轮全量回归 `cargo test --workspace --locked` | **327 passed / 0 failed**（新增 10 项：`pact-ref-spec` 4、`pact-http` 4、e2e 2） |
| [x] | 第 77/78 轮 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` | 均通过（退出码 0） |
| [x] | 推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、`cargo audit`） | 无新增依赖；生产代码无 `unsafe`/网络/进程/文件写；`cargo audit` 0 vulnerability / 2 unmaintained（沿用）；结论写入 `design-change.md` 第 77/78 轮 |
| [x] | 第 83 轮全量回归 `cargo test --workspace --locked` | **338 passed / 0 failed**（新增 11 项：`pact-http` 单测 9、e2e 2；含 `config` 空 paths 拒绝） |
| [x] | 第 83 轮 `cargo fmt --all -- --check`、`cargo clippy -p pact-http --all-targets --locked -- -D warnings` | 均通过（退出码 0） |
| [x] | 第 84 轮全量回归 `cargo test --workspace --locked` | **349 passed / 0 failed**（新增 11 项：`ref_reload_e2e` 7、`pact-http` 单测 3、CLI e2e 1） |
| [x] | 第 84 轮 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` | 均通过（退出码 0） |
| [x] | 第 84 轮推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、审计文件路径/权限、ACL 绕过） | 无新增依赖；文件写仅配置源读取与 `<data_dir>/ref-reload.jsonl` append+fsync，无 `unsafe`/网络/进程副作用；控制面要求认证 + `{domain}/*` ACL，审计失败 fail-closed；`cargo audit` 结论沿用 |
| [x] | 推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、`cargo audit`） | 无新增依赖；新增代码只读配置路径并解析 YAML，无 `unsafe`/网络/进程写；`cargo audit` 结论沿用 |
| [x] | 第 86 轮全量回归 `cargo test --workspace --locked` | **356 passed / 0 failed**（新增 7 项：`pact-analysis` digest 5、`pact-cli` lint 单测 1、`ref_lint_e2e` 1） |
| [x] | 第 86 轮 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` | 均通过（退出码 0） |
| [x] | 第 86 轮推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、`cargo audit`） | 无新增包（`pact-analysis` 仅启用 workspace 既有 `serde_json`/`sha2`）；`digest.rs` 无 `unsafe`/网络/进程/文件 IO，CLI 只读声明文件；`cargo audit` 0 vulnerability / 2 unmaintained（沿用） |
| [x] | 第 87 轮全量回归 `cargo test --workspace --locked` | **363 passed / 0 failed**（新增 7 项：`ref_digest_e2e` 5、`pact-http` 单测 2——`digest_mismatch` 与 `ref_digest_mode` 解析） |
| [x] | 第 87 轮 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` | 均通过（退出码 0） |
| [x] | 第 87 轮推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、`cargo audit`） | 无新增依赖；新增代码只读配置与绑定快照、计算纯函数摘要，无 `unsafe`/网络/进程/新文件写；`enforce` 在引擎前拒绝、无副作用；`cargo audit` 结论沿用 |
| [x] | 第 88 轮全量回归 `cargo test --workspace --locked` | **373 passed / 0 failed**（新增 10 项：`pact-ref-spec` 7、`pact-wasm` 3）；Node 测试 8 passed（含 `refBindings` 用例） |
| [x] | 第 88 轮 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、wasm32 clippy | 均通过（退出码 0）；`ci-smoke` 通过（raw 557852 B / gzip 167138 B） |
| [x] | 第 88 轮跨仓库：vendored 快照同步与 `pact-ref` 回归 | `scripts/sync-spec.sh --source … --check` 输出 `vendored spec is in sync`；`pact-ref` `fmt`/`clippy`/`test` 通过，**57 passed / 0 failed**；提交 `9be7cbd` 已推送，CI `41166710183` / Security `41166710038` 均成功 |
| [x] | 第 88 轮推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、`cargo audit`） | 新增依赖 `pact-wasm → pact-ref-spec` 为纯 crate（只依赖 serde，无 IO/网络/`unsafe`）；WASM 仅接收宿主注入 JSON，无新增文件 IO/进程/网络；`cargo audit` 0 vulnerability / 2 unmaintained（沿用） |
| [x] | 第 88 轮 CI（提交 `5f73772`） | CI `41166320702` **success**（audit / build-test-fmt-clippy / wasm 三个 job 通过） |
| [x] | 第 89 轮定向验收：真实二进制 `pact ref digest` / `ref lint`（含 `--tombstones`、`--strict`）/ 摘要漂移 | `ref digest` 输出 `sha256:19fe1f29…bdcbfb` 与 e2e golden 一致；`ref lint` → `errors=0 warnings=0` exit 0；`--strict` exit 0；篡改摘要后 `ref.bindings.digest_mismatch` exit 1 |
| [x] | 第 89 轮回归基线 `cargo test -p pact-cli --test ref_lint_e2e --locked` | **3 passed / 0 failed**（无代码变更，全量回归沿用第 88 轮 373 passed） |
| [x] | 第 89 轮推送前安全审视 | 仅新增 Markdown 技能与文档修改，无代码、无依赖、无网络/进程/文件写变化；`cargo audit` 0 vulnerability / 2 unmaintained（沿用） |
| [x] | 第 89 轮提交（`e88ee28`） | 4 files changed（新增 SKILL.md 386 行、`design-change.md`、`declaration-digest.md` 纠偏、`todo.md`）；尚未推送 |
| [x] | 第 90 轮契约校验：`yaml.safe_load` + `$ref` 遍历 | paths 为 `/healthz`、`/readyz`、`/v1/resolve`、`/v1/describe`；schemas 5 项；`all $ref resolve OK` |
| [x] | 第 90 轮回归基线：无代码变更 | 沿用第 88 轮全量 **373 passed / 0 failed**；`pact-ref` 仅文档改动 |
| [x] | 第 90 轮推送前安全审视 | 无新增依赖、无代码、无网络/进程/文件写变化；设计层面默认拒绝、凭据不入请求体、审计 fail-closed；`cargo audit` 结论沿用 |

## 已完成归档区

| 完成日期 | 事项 | 依据 | 证据 |
|---|---|---|---|
| 2026-10-31 | 物引治理 SKILL：六主题使用者流程（绑定/卡片/tombstone/可见性/摘要/reload）、适用闸门、不变量、lint/digest 闭环、输出模板与边界 | FR27–FR34 / 治理 §13 / `manual.md` §7.8–§7.14 | `skills/pact-thingref-governance/SKILL.md`；真实二进制 digest golden 一致、lint 0/0、`--strict` 0、漂移 exit 1；`ref_lint_e2e` 3 passed；`design-change.md` 第 89 轮 |
| 2026-10-31 | WASM 本地入口校验与绑定注入：入口语义抽为 `pact-ref-spec::validate_entry`，REST 委托、WASM 复用；宿主 JSON 注入 RefBindings | FR30 / D50 | `pact-ref-spec` 7 单测、`pact-wasm` 3 单测、Node 8 项、wasm32 clippy、`ci-smoke`；373 passed；`pact-ref` 57 passed；提交 `3934999`（跨仓库 `9be7cbd`，CI 成功）；CI `41166320702` 成功 |
| 2026-10-31 | `declarationDigest` v1 第二步：`[ref_digest] mode = off/warn/enforce` 运行期比对（`POST /domains` 进入引擎前；`enforce` → `400 DeclarationDigestMismatch`） | FR34 / D49 第二步 / `spec/ref/v1/declaration-digest.md` | `pact-http` 单测 2（三模式/边界、配置解析）；`ref_digest_e2e` 5；363 passed；`fmt`/`clippy` 通过；CI `41163741023` 成功；提交 `505e81d` |
| 2026-10-31 | `declarationDigest` v1 第一步：语义接口摘要工具与 lint 漂移比对 | FR34 / D49 / `spec/ref/v1/declaration-digest.md` | `pact-analysis` 单测 5（含 golden）、`pact-cli` 单测 1、`ref_lint_e2e` 1；356 passed；`fmt`/`clippy` 通过；CI `41162194383` 成功；`2bffcd8` |
| 2026-10-30 | B0：ContractCard 运行期加载、版本不可变与归档钉版并集 | FR32 / D47 | `pact-http` 单测 + e2e；338 passed；`fmt`/`clippy` 通过；`4b42778` |
| 2026-10-30 | B1：RefBindings/ContractCard 显式重载（`POST /ref/reload` + CLI，`dryRun`/审计/ACL/原子快照） | FR33 / D48 | 单测 3 + `ref_reload_e2e` 7 + CLI e2e 1；349 passed；`fmt`/`clippy` 通过；`a514380` |
| 2026-10-28 | ThingRef 写入路径校验（三态、URI profile、RefBindings 策略） | FR30 / D45 | `pact` `9893541`；317 passed；`pact-ref` `b8e759c` 46 passed |
| 2026-10-28 | 进度台账纪律（`todo.md` + `AGENTS.md`） | `AGENTS.md` | `9893541` |
| 2026-10-28 | 可发现性 lint（5 个 warning code）与真实二进制 e2e | FR29 / D44 | `eb86654`；309 passed |
| 2026-10-28 | 推送前安全审视（可发现性 lint 批次） | `AGENTS.md` | `design-change.md` 第 75 轮 |
| 2026-10-27 | 物引静态检查 `pact ref lint` | FR27 / D41 | 已发布 |
| 2026-10-27 | 物引可见性过滤（`read-gate`/`domain`/`hidden`） | FR28 / D42 | 已发布 |
| 2026-10-27 | 视图级读审计（`kind=view`，fail-closed） | D43 | 已发布 |
| 2026-10-27 | 保留与 tombstone（retention / deletionPolicy / Tombstone schema） | FR29 / D44 | 已发布 |
