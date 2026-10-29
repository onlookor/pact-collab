# TODO（进度台账）

> 纪律见 `AGENTS.md`：每轮开始先读本文件确认事项与状态；每轮结束更新状态、验证证据与提交哈希。
> 状态：`[ ]` 待做 · `[~]` 进行中 · `[!]` 阻塞 · `[x]` 完成。完成项移入末尾“已完成归档区”。
> 依据列只引用权威文档编号；新编号先登记到 `docs/architecture.md` 附录 A 再使用。
> 修复顺序以本文件为准；调整、降级或拆分必须写明原因。

当前状态（2026-10-29，第 82 轮）：P0 入口校验与 P1 的 `pinning` / `onUnknownScheme` / `value` 已落地并全量回归通过；ContractCard 方案已确认并登记编号——B0 卡片加载与版本不可变（FR32/D47）、B1 显式重载（FR33/D48），均待实现；`declarationDigest`（推荐语义接口摘要 + 首版默认 `warn`）仍待确认。WASM 入口、`refd` 与动态阶段未开始。下一轮按 B0 → B1 → P2 → P3 顺序推进。

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
| [ ] | `pact-wasm` 本地入口校验：`host.rs` 直接构造 `ThingRef`，未做 URI profile / 绑定校验 | FR30 / `thingref-governance.md` §8.2 | 复用 `pact-ref-spec` 纯函数（WASM 可承载）；需先定 profile 从何处注入 |
| [x] | `pact-runtime` 种子/生产路径确认继承 REST 校验 | FR30 | `pact-runtime` 经 `RestClient` 调 REST；`runtime_e2e` 5 passed（含旧 Identifier 域需 `allowIdentifier: true` 的回归修正） |

### P1 归档钉版与运行期绑定策略

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [x] | `pinning` 运行期强制：`required` 在写入时、`required-at-archive` 在 consume 进入 Archive 前要求钉版摘要（FR31/D46） | FR31 / D46 / `uri-profile.md` §4 | 单测 + e2e：写入/consume 缺摘要 → `400 ThingRefUnpinned`；钉版可归档 |
| [x] | `onUnknownScheme` 运行期生效（`reject` / `treat-as-identifier` / `allow`） | FR31 / D46 / `uri-profile.md` | 单测三分支 + `treat-as-identifier` 未放行 Identifier 被拒 |
| [x] | `RefRule.value`（`none`/`uri`/`identifier`）语义落地 | FR31 / D46 / `ref-bindings.schema.json` | 入口强校验 + 单测；`required=true` 与 `value=none` 由绑定校验拒绝 |
| [ ] | B0：ContractCard `artifact.digestPolicy` / `reference.pinning.requiredForArchive` 运行期加载；daemon 新增 `[ref_contracts].paths`，按 `contract_id()` 建索引；声明 `contract` 的部署缺卡即拒绝启动；重复 `id@version` 即拒绝启动 | FR32 / D47 | 单测：加载/索引、按数据触发、重复 `id@version`、钉版并集只加严；e2e：缺卡启动失败、卡片钉版阻断未钉版 consume（`ThingRefUnpinned`） |
| [ ] | B1：显式重载 `POST /ref/reload` + `pact ref reload`；卡片与绑定同事务、原子换索引、`dryRun` 影响报告、`ref-reload.jsonl` 审计、ACL 覆盖绑定域；不做文件监听 | FR33 / D48 | 单测：事务失败不生效、并发 `409 RefReloadBusy`、审计失败拒绝、ACL；e2e：`dryRun` 报告、成功重载后新规则生效、失败保持旧状态 |
| [!] | `declarationDigest` 漂移检测：当前只校验格式，不参与匹配 | FR27 边界 / `ref-bindings.schema.json` | 方案与收益/代价见治理 §13.4.2；推荐语义接口摘要（C）+ 工具/lint 先行，运行期首版默认 `warn`、`enforce` 可选（误算不直接阻塞 `load_domain`，见 §13.4.3）；**待用户确认**摘要对象、是否分两步、摘要字符串格式 |

### P2 跨仓库与快照

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [x] | `pact-ref` vendored 快照同步（第 77 轮：`HARD_MAX`、Identifier 文法、userinfo、`InvalidIdentifier`；第 78 轮：`merged_rule`、`pinned_digest`、`classify_identifier`、新错误） | `AGENTS.md` 跨仓库纪律 | `scripts/sync-spec.sh --check` 通过 |
| [x] | `pact-ref` 回归：vendored 快照更新后跑 `cargo fmt/clippy/test` | 同上 | 第 77 轮 46 passed；第 78 轮 **50 passed / 0 failed**，`fmt`/`clippy` 通过 |
| [!] | `pact-ref` conformance：补入口校验用例（缺失/`null`、userinfo、Identifier 文法、`maxLength`） | `conformance/` | 阻塞：`conformance/runner` 仍是骨架（不执行用例）；待 runner 落地后补 |

### P3 治理平面与动态阶段

| 状态 | 事项 | 依据 | 验收方式 |
|---|---|---|---|
| [!] | `refd` resolve/describe 端点 | `thingref-governance.md` §13.1 阶段 1 待做 | 阻塞于端点契约与鉴权设计；属治理平面仓库 |
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
| [x] | 第 82 轮：登记 `FR32`/`D47`（B0）与 `FR33`/`D48`（B1）；`requirements.md` 新增两节、`requirements-change.md` 第 40 轮；`design.md` §0 新增 D47/D48；治理 §13.4.1/§13.5 标注已确认与编号；`architecture.md` 附录 A 登记 | 编号先登记后引用；仅文档，无需回归 |

### 验证与交付

| 状态 | 事项 | 验收方式 |
|---|---|---|
| [x] | 第 77 轮全量回归 `cargo test --workspace --locked` | **317 passed / 0 failed**（50 个 test binary，含 `runtime_e2e` 5 passed） |
| [x] | 第 78 轮全量回归 `cargo test --workspace --locked` | **327 passed / 0 failed**（新增 10 项：`pact-ref-spec` 4、`pact-http` 4、e2e 2） |
| [x] | 第 77/78 轮 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` | 均通过（退出码 0） |
| [x] | 推送前安全审视（凭据、`unsafe`、网络/进程/文件写、新增依赖、`cargo audit`） | 无新增依赖；生产代码无 `unsafe`/网络/进程/文件写；`cargo audit` 0 vulnerability / 2 unmaintained（沿用）；结论写入 `design-change.md` 第 77/78 轮 |
| [x] | 第 78 轮提交与推送，回填提交哈希 | `pact` `f2c939f`（main）；`pact-ref` `8442c2d`；均已推送 |

## 已完成归档区

| 完成日期 | 事项 | 依据 | 证据 |
|---|---|---|---|
| 2026-10-29 | `value` / `onUnknownScheme` / `pinning`（写入与归档）运行期强制 | FR31 / D46 | `pact` `f2c939f`；327 passed；`pact-ref` `8442c2d` 50 passed |
| 2026-10-28 | ThingRef 写入路径校验（三态、URI profile、RefBindings 策略） | FR30 / D45 | `pact` `9893541`；317 passed；`pact-ref` `b8e759c` 46 passed |
| 2026-10-28 | 进度台账纪律（`todo.md` + `AGENTS.md`） | `AGENTS.md` | `9893541` |
| 2026-10-28 | 可发现性 lint（5 个 warning code）与真实二进制 e2e | FR29 / D44 | `eb86654`；309 passed |
| 2026-10-28 | 推送前安全审视（可发现性 lint 批次） | `AGENTS.md` | `design-change.md` 第 75 轮 |
| 2026-10-27 | 物引静态检查 `pact ref lint` | FR27 / D41 | 已发布 |
| 2026-10-27 | 物引可见性过滤（`read-gate`/`domain`/`hidden`） | FR28 / D42 | 已发布 |
| 2026-10-27 | 视图级读审计（`kind=view`，fail-closed） | D43 | 已发布 |
| 2026-10-27 | 保留与 tombstone（retention / deletionPolicy / Tombstone schema） | FR29 / D44 | 已发布 |
