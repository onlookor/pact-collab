# 需求变更记录

> 历史记录保留当时的编号与措辞；现行编号约定见 `docs/architecture.md` 附录 A。

## 2026-10-31（第 45 轮，`refd` OIDC 身份验证，FR35/D51）

- **新增 FR35（D51）**：`refd` 作为 OIDC 资源服务器验证 IdP 签发的 JWT（JWKS 验签），只做验证、不做授权服务器或 discovery。`[auth] mode = "oidc"` 时必需 `[auth.oidc]`，字段与缺省值见设计 §9.18；配置非法拒绝启动。
- **验签约束**：算法白名单 `RS256`/`ES256`/`EdDSA`，拒绝 `none`/`HS*` 与 `crit`；`kid` 匹配、无 `kid` 仅唯一密钥可接受；校验 `iss`/`aud`/`exp`/`nbf`；`sub` 为 actor，`scope` claim 映射端点 scope。
- **JWKS 与失败语义**：有界缓存（≤ 64 密钥、按 TTL）、未知 `kid` 单次强制刷新且 30s 限频；拉取/解析/验签失败一律 `401` 并带 `WWW-Authenticate: Bearer realm="pact-ref"`，不区分原因。`/readyz` 仍只反映审计 sink 健康。
- **编号登记**：`FR35`/`D51`/`[auth.oidc]` 及子项已于第 92 轮（`docs/architecture.md` 附录 A）登记后再引用。
- **测试**：`ref-auth` 单测 11 项、`refd` 集成测试 3 项（200、401+`WWW-Authenticate`、403）；`pact-ref` 全量 99 passed / 0 failed；`fmt`/`clippy`/`cargo audit` 通过。
- 其余 FR1–FR34、NFR1–NFR7 不变。

## 2026-10-31（第 44 轮，WASM 本地入口校验，D50）

- **FR30 边界扩到 WASM**：`pact-wasm` 本地宿主不经过 REST，但 `prepare_seed` / `prepare_produce` 在进入 kernel 前复用同一 `pact-ref-spec::validate_entry` 纯函数；宿主以 JSON 注入 RefBindings（`refBindings` 选项 / `PACT_WASM_REF_BINDINGS`），未注入时只做结构校验。JS/WASM 无法表达“字段缺失”，省略与 `null` 都映射为 `∅`，因此 WASM 路径不产生 `ThingRefMissing`。
- **FR21 能力范围同步**：WASM 在写入前执行票面结构规则与绑定策略；ContractCard 加载、归档钉版与 Archive 检查仍只在 REST 边界。
- **FR31 边界同步**：WASM 写入侧执行 `value` / `onUnknownScheme` / `pinning: required`，但不加载卡片、不做 consume 前 `required-at-archive`。
- **FR30 实现状态**：补 2026-10-31 WASM 侧完成；未生效项清单不变（`declarationDigest` 由 FR34 承接）。
- 其余 FR1–FR29、FR31–FR34、NFR1–NFR7 不变。

## 2026-10-31（第 43 轮，`declarationDigest` v1 第二步：运行期比对）

- **FR34 第二步实现**：新增可选 `[ref_digest].mode = "off" | "warn" | "enforce"`（缺省 `warn`；整段可省，未知键/未知模式拒绝启动）。daemon 在 `POST /domains`（`load_domain`）授权后、进入引擎前解析声明并计算摘要，与该域当前绑定 `metadata.declarationDigest` 比对。
- **行为边界**：`off` 不比较；`warn` 写 stderr 警告并照常加载；`enforce` 仅“摘要存在且不一致”返回 `400 DeclarationDigestMismatch`（`detail` 含 `domain`/`expected`/`actual`/`digestVersion`），拒绝时域未装载、无事件写入。无绑定、绑定未记录摘要或声明解析失败 → 不比较；解析失败保留既有 `Syntax`/`WellFormedness`。启动恢复与 `ref reload` 不比较。
- **编号登记**：`[ref_digest]` / `[ref_digest].mode` 与错误码 `DeclarationDigestMismatch` 先登记于 `docs/architecture.md` 附录 A（第 87 轮）再引用。
- **FR30 未生效项收敛**：`declarationDigest` 的 lint 与运行期比对均由 FR34 实现，不再列为未生效。
- **测试**：`pact-http` 单测覆盖纯函数 `digest_mismatch` 的三模式与边界；新增 `ref_digest_e2e` 5 项（匹配放行、enforce 拒绝且无加载副作用、warn/off、缺失摘要与外来域、解析失败保留既有错误码）；全量 `cargo test --workspace --locked` 363 passed / 0 failed。
- 其余 FR1–FR33、NFR1–NFR7 不变。

## 2026-10-31（第 42 轮，`declarationDigest` v1 第一步实现）

- **新增 FR34（D49）**：域声明语义接口摘要 `declarationDigest` v1。摘要对象 = 域名 + `types` + `preconditions` + `gates`（含条件表达式）+ `seed_authority`；规范化冻结在 `spec/ref/v1/declaration-digest.md`（UTF-8 紧凑 JSON、固定字段顺序、字符串字典序、条件自定文法文本），摘要为 `sha256:` + 64 位小写 hex。
- **第一步实现**：`pact ref digest <file.pact> [--name N] [--json]`；`pact ref lint --declaration` 计算并比对绑定摘要——缺失 warning `ref.bindings.digest_missing`（`--strict` 失败），已记录且不一致 error `ref.bindings.digest_mismatch`（退出 1）。实现落在 `pact-analysis::digest` 纯函数，`pact-ref-spec` 与依赖方向不变。
- **FR27 边界同步**：lint 由“不校验摘要内容”改为按 v1 规范计算并比对；检查项由“摘要缺失”扩为“缺失或不一致”。
- **FR30 未生效项收敛**：`declarationDigest` 的语义比对已由 FR34 第一步实现；运行期比对（`[ref_digest].mode`）仍在 D49 第二步，未生效。
- 其余 FR1–FR33、NFR1–NFR7 不变。

## 2026-10-30（第 41 轮，B1 显式重载实现）

- **FR33 实现**：`POST /ref/reload`（`{"dryRun": bool}`）与 CLI `pact ref reload [--dry-run] [--json]` 落地；卡片与绑定同事务重新读取、解析并规范校验 → 校验全部 `contract` 引用 → 计算新增归档阻断报告 → 写 `<data_dir>/ref-reload.jsonl` 审计（append+fsync）→ 以 `RwLock<Arc<VisibilityProfiles>>` 原子换快照。任一步失败保持旧快照；`dryRun` 在审计后停止且 `applied=false`。
- **错误面**：`400 RefReloadInvalid`、`409 RefReloadBusy`、`403 Forbidden`（ACL 未覆盖当前或新绑定域）、`401 Unauthorized`（配置身份后未认证）；实现防御分支 `500 RefReloadUnavailable`（未注入重载配置）、`500 RefReloadFailed`（影响预检失败）、`500 RefReloadAuditFailed`（审计写失败，覆盖原错误）。认证失败与并发冲突不写审计。
- **FR33 状态**：由“未实现”改为“已实现（2026-10-30，B1）”；FR32 状态与边界描述同步。
- **FR30/FR32 过期措辞修正**：`docs/requirements.md` 中“卡片加载与显式重载均待实现”“显式重载仍待 FR33”等改为已实现；FR30 未生效项收敛为仅剩 `declarationDigest`。
- 其余 FR1–FR32、NFR1–NFR7 不变。

## 2026-10-29（第 40 轮，ContractCard 加载与显式重载登记）

- **新增 FR32（B0）**：ContractCard 运行期加载与版本不可变。可选 `[ref_contracts].paths` 在 daemon 启动时解析、规范校验并按 `contract_id()`（`metadata.id@version`）建索引；任一 RefBindings 有效规则声明 `contract` 时，必须配置且每个契约可解析，否则拒绝启动；同一 `contract_id()` 重复（无论内容）即拒绝启动。consume 的有效钉版要求 = 绑定 `pinning` ∪ 卡片 `reference.pinning.requiredForArchive` / `artifact.digestPolicy: required-at-archive`，卡片只加严。`artifact.kind`/`mediaType` 与 `lifecycle` 运行期检查不在范围。
- **新增 FR33（B1）**：ContractCard 与 RefBindings 显式重载。REST `POST /ref/reload`（`dryRun` 可选）与 CLI `pact ref reload`，不提供 MCP 工具；卡片与绑定同事务校验后原子换索引，失败保持旧状态；并发 `409 RefReloadBusy`，校验失败 `400 RefReloadInvalid`；审计写 `<data_dir>/ref-reload.jsonl`（`ts`/`by`/`dryRun`/`outcome`/每文件 `sha256`），审计写失败拒绝重载；ACL 要求覆盖当前与新的绑定索引中每个域的 `{domain}/*`。不做文件监听、SIGHUP、自动回滚或按域灰度。
- **回溯性加严**：按 FR31/D46 在 consume 时按当前规则判定；换绑到更严卡片版本会让存量未钉版 token 不可归档，由 `dryRun` 影响报告与发布说明承担，不改变判定时机。
- **FR30/FR31 边界同步**：ContractCard 运行期加载由 FR32/FR33 承接，仍待实现；`declarationDigest` 未定，仍待决策。
- 其余 FR1–FR31、NFR1–NFR7 不变。

## 2026-10-29（第 39 轮，引用形态、未知 scheme 与钉版运行期强制）

- **新增 FR31**：把 RefBindings 的 `value`、`onUnknownScheme`、`pinning` 从“仅 lint 生效”接到运行期。
  - `value` 约束非空票面形态（`none`/`uri`/`identifier`），`required=true` 与 `value=none` 互斥；
  - `onUnknownScheme` 以“不在有效 `schemes` 白名单”判定未知，支持 `reject`（缺省）/`allow`/`treat-as-identifier`；
  - `pinning: required` 在写入时、`required-at-archive` 在 consume（进入 Archive）前要求 URI 携带 `sha256` 摘要（fragment 或 OCI 形式）；`∅` 不受约束，Identifier 不算钉版；失败返回 `400 ThingRefUnpinned`。
- **FR30 收窄未生效项**：`onUnknownScheme`、`value`、`pinning` 已由 FR31 生效；`declarationDigest` 与 ContractCard 的 `artifact.digestPolicy` / `reference.pinning` 运行期加载仍未落地，登记于 `todo.md`。
- **边界**：consume 检查只读取继承出的 `pinning`，不重跑写入期规则，避免历史票面因规则收紧而无法归档；ContractCard 运行期加载需先定卡片来源与配置（独立事项）。
- 其余 FR1–FR30、NFR1–NFR7 不变。

## 2026-10-28（第 38 轮，ThingRef 写入路径校验）

- **新增 FR30**：`thing_ref` 在 REST 写入边界（`POST .../seeds`、`POST .../tokens`）完成三态判定（缺失 → `400 ThingRefMissing`；显式 `null` → `∅`；字符串 → 引用）、URI profile 结构校验（控制字符、裸路径、密码型 userinfo、长度）、Identifier 文法与 RefBindings 策略（`required` / `allowIdentifier` / `schemes` / `maxLength`）。成功写入规范化后的值（scheme 小写）。
- **边界**：校验在接口层完成，引擎/存储/`pact-core` 仍不解析票面；CLI 与 MCP 经 REST 继承；WASM 本地宿主与 `pact-runtime` 进程内路径不在本次范围（WASM 单列事项）。
- **未生效项**：`onUnknownScheme`、`RefRule.value`、`declarationDigest` 与归档钉版运行期强制仍待做，登记于 `todo.md` P1。
- **修正规范层两处偏差**：`git+ssh://git@example.com/...`（纯用户名 userinfo）不再被误拒；Identifier 严格执行 `1*(ALPHA/DIGIT/_/-/./:/@/#)`，空格与 `?` 等字符不再被接受。
- 其余 FR1–FR29、NFR1–NFR7 不变。

## 2026-10-27（第 37 轮，引用可发现性 lint）

- **FR29 扩展**：`pact ref lint` 对 `required=true` 的绑定增加可发现性静态检查，共 5 个 warning code：`ref.discoverability.required_hidden`、`view_read_path_missing`、`no_reader`、`retention_missing`（非归档钉版的无 `lifecycle` 契约）、`deletion_without_tombstone`（`deletionPolicy=delete`）。`--strict` 将 warning 视为失败。
- **FR27 同步**：检查项补充“`required=true` 类型的可发现性（读取路径、视图暴露、保留证据与删除可区分性）”；输入、输出与退出码不变。
- **边界**：只证明静态路径存在，不保证运行期一定可读；不检查内容删除、tombstone 发布或解析可达。
- 其余 FR1–FR29、NFR1–NFR7 不变。

## 2026-10-27（第 36 轮，引用保留与 tombstone）

- **新增 FR29**：ContractCard `lifecycle` 的保留期与删除证据成为可静态验收的契约声明。`retention` 只接受 `permanent` 或正 ISO 8601 duration；`deletionPolicy` 只接受 `retain` / `delete` / `tombstone-then-delete`；`tombstone-then-delete` 必须有 URI 形式的 `tombstoneRef`，其余策略不得声明；归档钉版禁止 `delete`。
- **Tombstone 记录**：新增 `pact.ref/v1` 的 `Tombstone` 规范对象（`metadata.id`、`spec.contract/ref/reason/deletedAt/retainUntil/replacedBy/note`），规范时间戳、替换关系与说明字段；tombstone 由域作者以正常 seed/produce 产生。
- **FR27 扩展**：`pact ref lint` 新增 `--tombstones`、`tombstones` 计数与 tombstone 交叉检查；归档钉版契约缺少 `lifecycle` 时告警。
- 其余 FR1–FR28、NFR1–NFR7 不变。

## 2026-10-27（第 35 轮，视图级读审计）

- **触发**：继续推进物引治理路线，补齐可见性阶段的遗留项——视图级读审计。
- **FR17 扩展**：`read_audit = true` 同时覆盖 Counter/Archive 视图读取。同一审计流新增 `kind=view` 记录：`view`（counter/archive）、`type_filter`、按类型聚合的 `types[{type,tokens,refs_visible,refs_filtered}]`；只记暴露计数，不记录 `thing_ref` 值。`participant`/`token_id` 改为可选以承载匿名视图；旧记录无 `kind` 按 `read` 解析。写审计失败时视图以 `500` 拒绝（fail-closed）。
- **FR28 边界更新**：视图级审计不再列为待做；明确其与可见性过滤、Rule-R 的边界。
- 其余 FR1–FR27、NFR1–NFR7 不变。

## 2026-10-27（第 34 轮，物引可见性过滤）

- **触发**：继续推进物引治理路线，进入可见性阶段；用户要求继续推进。
- **新增 FR28**：Counter/Archive 的 `thing_ref` 默认 `read-gate`；`domain` 显式公开、`hidden` 永不返回；显式 profile 来自 daemon `[ref_bindings].paths` 指向的 RefBindings；令牌/签名模式下视图要求身份，匿名 `401`；加载失败 fail-closed。
- **边界**：只过滤视图票面，不改变 token 存在性；`GET /tokens/{id}` 仍由 read gate 授权；视图级审计未在本次实现。
- 其余 FR1–FR27、NFR1–NFR7 不变。

## 2026-10-27（第 33 轮，物引静态检查命令）

- **触发**：用户要求实现 `pact ref lint` 静态检查命令。
- **新增 FR27**：离线检查 RefBindings、ContractCard 与 `.pact` 声明的静态一致性；支持绑定/契约目录展开、声明类型对照、契约引用与 scheme 一致性、策略告警、契约状态与未使用检查；`--json` 报告与 `--strict` 退出码。
- **边界**：不解析具体 `ThingRef` 存在性、不访问网络、不进入引擎；不校验 `declarationDigest` 内容。
- 其余 FR1–FR26、NFR1–NFR7 不变。

## 2026-10-26（第 32 轮，用户向文档定位表述）

- **触发**：用户指出当前 README / 手册以内部实现细节（如“不重写 loop”“治理插件”）描述 PACT，外部读者无法判断它是什么、归入哪一类、与常见工具相比有何异同。
- **FR14 增补（定位表述）**：所有面向用户的 README 与手册开篇必须给出定位（声明式协同层，不是 agent 框架或工作流执行器）、归类、与常见工具的异同及不适用边界；对比只陈述可核对的能力差异，不贬低他者；某个宿主的装配细节不得当作产品定位。
- **范围**：只改用户向文档的表述与组织（`README.md` / `README.zh-CN.md` / `docs/manual.md` / `examples/README*.md` / `docs/authoring-contract.md` 引言）；不改范式语义、不改 FR / NFR 编号、不改引擎代码。
- 其余 FR1–FR26、NFR1–NFR7 不变。

## 2026-10-25（第 31 轮，文档编号清理）

- **触发**：用户要求清理“对话中自创、文档中无解释使用”的简写与编号（此前 `h3c`、`L0–L4` 等）。
- **FR16**：e2e 测试中的裸 `P1/P2/P3` 改为“生产优先 / 汇消费 / 等待三档策略”，不再引入编号。
- **FR19**：TokenId 唯一性引用由裸 `Q4` 改为 `W-Q4`（WASM 待决问题命名空间）。
- **FR20**：单实例作用域的 `G1/D27` 改为 `D12/D27`（`G1` 在契约中另有含义，跨文档引用改用设计决策编号）。
- **FR21**：`Q7` / `S2` 分别改为 `W-Q7` / `W-S2`，与 `docs/wasm-local-spike-feasibility.md` 的命名空间一致。
- **无需求语义变化**：仍为 FR1–FR26、NFR1–NFR7 的既有内容。

## 2026-10-25（第 30 轮，控制面认证、域级授权与审计）

- **安全审视闭环（#3/#4/#5）**：补齐状态交接控制面的认证、授权与审计边界，修正 #6/#11 文档表述。
- **FR26 认证**：配置了任何身份（`[auth.tokens]` 或 `[auth.keys]`）后，`handoff` / `freeze` / `activate` / `active` 必须可认证；信任声明模式（两者皆空）免身份。既有生命周期/查询/压缩端点行为不变（令牌仅约束四类动作端点）。
- **FR26 授权**：ACL 启用时 `handoff` 要求源实例键、目标实例键与域通配键 `{d}/*`（交接改写域级 `declaration.pact`）；`PUT /active` 要求 `{d}/*`；`GET /active` 与 `freeze` 按实例键授权。
- **FR26 审计**：`.sealed` 改为 JSON `{version, reason, ts, by}`，`active.json` 增加 `ts`/`by`；`GET /domains/{d}/active` 返回两字段。旧版纯文本 `.sealed` 与无审计字段的 `active.json` 保持可读。
- **文档更正（#6/#11）**：手册不再声称“未配置 ACL 时令牌约束全部端点”；MCP 模块说明由“缺省 `default`”改为“缺省活动实例”。
- **测试**：新增控制面认证/审计端到端用例；交接 ACL 用例改为验证“源+目标键齐备但缺 `{d}/*` 仍拒绝”。全量 245 项通过。
- 其余 FR1–FR25、FR26 既有语义与 NFR1–NFR7 不变。

## 2026-10-25（第 29 轮，FR26 冻结完整性修复）

- 安全审视发现：冻结源可在 `unload` 后经重载/实例化绕过冻结（内存解冻、磁盘标记仍在，重启后又变回冻结）。
- **收紧 FR26 冻结语义**：已冻结实例的重载/实例化在无 `force` 时返回 `InstanceFrozen`；`force` 重置才清除 `.sealed` 并解冻；交接目标若残留 `.sealed` 视为已存在（`InstanceExists`）并拒绝，避免覆盖审计。
- 新增回归测试：卸载后无 force 重载/实例化被拒 + 重启后状态与冻结保持 + force 解冻可持久；目标残留 `.sealed` 时交接失败且无副作用。
- 其余 FR26 语义与 FR1–FR25、NFR1–NFR7 不变。

## 2026-10-25（第 28 轮，FR26 显式状态交接与活动实例别名）

- **新增 FR26**：显式状态交接（冻结源实例 → 以新声明创建目标 → 迁移 Counter/Archive/幂等表 → 可选切换活动实例 → 源只读），并在域双键之上引入域级活动实例别名。
- **语义裁决**：`Imported` 为自足基线事件（schema v2）；恢复取最后基线；冻结持久化并允许幂等重放；兼容性下界只看 Counter（Archive 可保留已删除类型）；激活 best-effort；悬空指针 fail-fast；卸载活动实例清除指针；不提供原位保留状态的声明换版、双活或自动回滚。
- **接口面**：REST 三条新端点、CLI 四个子命令、MCP 四个工具。
- **同步修订**：FR17 版本契约补充 `Imported` v2 与读取上限；FR19 短路由语义改为「解析活动实例，无指针回退 default」，实现状态工具数改为 21；FR23 工具数 17 → 21。
- **既有条目**：FR1–FR25、NFR1–NFR7 其余语义不变。

## 2026-10-23（第 27 轮，FR25 npm 分发与安装解析修复）

- npm 发布：`dsh-plugin-pact@0.1.2`（同日依次发布 `0.1.0`、`0.1.1`）；README 增加 npm 版本徽章与 `Published` 说明。
- 安装解析实测：以 `dsh plugin --profile npm-probe add dsh-plugin-pact`（pnpm 11.28.5、`@deepseek-ai/dsh@0.2.1-rc.2`）安装，依赖解析为 `^0.1.1`，lockfile 记录 npm integrity，`--dump-config` 组合出“禁用原行 + 两个插入行”。
- 缺陷：`0.1.1` 的 `cordis.patch.yml` 仍指向迁出前的 `@pact/dsh-plugin`，启动时 `pact-agent-loop` 与 `dsh-pact-observe` 均报 `ERR_MODULE_NOT_FOUND`，插件未激活。
- 修复：patch 名称改为 `dsh-plugin-pact/loop` 与 `dsh-plugin-pact`；新增 `test/manifest.test.js` 断言 patch 名与包名一致；版本升至 `0.1.2`，8 个测试文件 49 项通过。
- 发布与复测：推送 `v0.1.2` 触发 `release.yml` 经 OIDC 发布成功；在全新 profile 执行 `add dsh-plugin-pact` 解析为 `^0.1.2`，`--dump-config` 正确组合，启动无未激活告警，`dsh-plugin-pact` 与 `/loop` 均可解析。
- 既有条目：FR1–FR25、NFR1–NFR7 语义不变。

## 2026-10-23（第 26 轮，FR25 交付位置变更）

- FR25 的实现位置由本仓库 `plugins/dsh/` 改为独立仓库 `onlookor/pact-dsh`，包名 `dsh-plugin-pact`（`0.1.0`）。
- 需求内容不变：observe/govern 两档、`pact-*` 选择性接管、可注入后端、审计最小化、govern fail-closed。
- 分发形态明确为公开 GitHub 仓库 + npm 包（发布待执行），并以 GitHub `dsh-plugin` topic 参与生态发现；不改变与 PACT 引擎的边界。
- 本仓库不再包含插件代码；WASM 宿主组装入口 `crates/pact-wasm/js/open-host.js` 仍属引擎侧资产。

## 2026-10-23（第 25 轮，FR25 WASM 后端 live）

- **验证范围**：以官方 `wasm-bindgen 0.2.127` 构建 `pact-wasm` Node 产物，新增 `crates/pact-wasm/js/open-host.js` 组装 `PactHost + FileStore`；在 DSH `pact-headless` profile 中以 `backend=wasm` 驱动 `pact-*` agent turn。
- **结果**：允许路径下 bash 实际执行（`isError:false`，输出 `PACT_TOOL_OK`）；`denyTools:["bash"]` 返回 `Error: pact: tool denied by policy`；全新实例（Counter 为空）在 `agent/pre-step` 被拒绝，会话仅 `turn/start → turn/end`。实例事件日志由插件运行时经 WASM 宿主创建。
- **FR25 配置补充**：新增可选 `wasmOptions`（纯对象），原样透传给 `openPactHost`；插件只校验其为对象。
- **语义澄清**：WASM 宿主的 `enabled` 返回 Rule-W 可生产选项数组，插件适配层收窄为布尔（非空即“当前可执行”）；不改变 PACT 语义。
- **边界**：验证使用确定性 mock provider（临时 profile，不入仓库）；插件退出时不显式关闭宿主，锁文件残留由 pid 存活检测回收。
- **既有条目**：FR1–FR25、NFR1–NFR7 语义不变。

## 2026-10-23（第 24 轮，FR25 真实模型门控复跑）

- **复跑环境**：用户提供 `DEEPSEEK_API_KEY`，以 `deepseek-official/deepseek-flash` 在 `pact-headless` profile 中驱动真实 agent turn。
- **结果**：允许路径 bash 返回 `PACT_TOOL_OK`；`denyTools:["bash"]` 返回 `Error: pact: tool denied by policy`；`fakeEnabled:false` 时 `turn/start` 后直接 `turn/end`，无模型请求。三条路径均与 mock 结果一致。
- **结论**：FR25 的 `pact-*` 门控在真实模型会话中成立；WASM 后端 live 仍未验证。
- **安全**：凭据仅经 0600 临时文件与环境变量使用，未落仓库、未回显，验证后删除。
- **既有条目**：FR1–FR25、NFR1–NFR7 语义不变。

## 2026-10-22（第 23 轮，FR25 真实会话门控验证）

- **验证范围**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 的 headless profile 中注册 `pact-demo` preset 与本地确定性 mock LLM，通过真实 `ctx.agents.create` 创建 `pact-*` agent 并驱动 turn。
- **结果**：`govern` 允许路径下 `bash` 正常执行；`denyTools:["bash"]` 时工具被拒（`pact: tool denied by policy`）；`fakeEnabled:false` 时 `agent/pre-step` 拒绝，turn 在模型调用前结束。
- **FR25 配置补充**：新增 `fakeEnabled`（默认 `true`），仅对 `backend=fake` 生效，用于预演未启用路径。
- **边界**：上述为真实 DSH 会话与真实钩子，但模型为本地 mock；真实模型供应商凭据下的复跑仍待用户提供凭据。WASM 后端产物未接线。
- **既有条目**：FR1–FR25、NFR1–NFR7 语义不变。

## 2026-10-22（第 22 轮，FR25 真实 DSH 装配验证与 patch 语义修正）

- **触发**：用户批准安装依赖并验证真实 DSH 装配。
- **装配结论**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 上创建 web profile 并安装 `plugins/dsh/`；`--dump-config` 显示原 `agent-loop` 行 `disabled: true`、新增 `pact-agent-loop`/`dsh-pact-observe`；启动后探针确认 `ctx.agentLoop instanceof PactAgentLoop`，且继承官方 `AgentLoop`、`Config` 同一。
- **FR25 表述修正**：DSH patch 只按 `id` 覆盖或禁用，`name` 仅作一致性校验、不能改写注册名；因此接管方式由“覆盖 `agent-loop` 行”修正为“禁用原行 + 插入 `pact-agent-loop`（同一 `agentLoop` 服务）”。`@deepseek-ai/dsh-agent-loop` 登记为 peer，由 DSH 运行时解析。
- **实现状态更新**：41 项 Node 测试仍通过；真实装配已验证，仍未在真实模型会话中触发 `pact-*` 门控（需模型凭据与 PACT 后端），WASM 后端产物未接线。
- **既有条目**：FR1–FR24、NFR1–NFR7 语义不变。

## 2026-10-22（第 21 轮，阶段 11 立项：FR25 DSH Agent Loop 集成）

- **触发**：用户要求推进 DeepSeek Harness 插件替换原有 agent loop；依据共享讨论（PACT 只接管控制流、不接管模型调用）与三项裁决（治理型 loop 替换 / 可注入后端 + 假实现 / `plugins/dsh/`）。
- **新增 FR25 DSH Agent Loop 集成（D37）**：仓库内 JS 包 `@pact/dsh-plugin`；`observe` 镜像 `session/event`（有界缓冲 + 可选 JSONL），`govern` 在 `agent/pre-step` / `tools/pre-execute` 做 PACT 门准入（`reject` / `deny`）；`cordis.patch.yml` 覆盖 `agent-loop` 行，`PactAgentLoop extends AgentLoop` 仅对 `pact-*` preset 安装 agent 级钩子，其余 agent 原样委托。
- **后端与边界**：`PactBackend` 契约 + `FakePactBackend` + 惰性 `WasmPactBackend`；不引入守护进程/端口/网络依赖，不改 Rust crate，不进入 `dist/`；审计只落 `type/seq/工具名/call id`，不落工具参数/结果。
- **失败策略**：`govern` fail-closed，`observe` fail-open 并留痕；`turn-stopping` 读失败不阻断收尾。
- **已知限制**：`resume` 首步不受门控；本轮不做 token 级工具映射、saga 补偿与 PACT 原生工具（阶段 12 评估）；依赖 DSH 预发布 API。
- **§6 范围外修订**：移除“DSH loop 治理”，改列为“DSH 团队级原生运行时、浏览器宿主与 NFR6 v2 远端模式”；DSH 集成范围由 FR25 限定为 observe/govern + `pact-*` preset。
- **实现状态**：已实现（2026-10-22）；`node --test` 41 项通过、`node --check` 全部通过；未安装 dsh/pnpm/wasm-bindgen，真实装配与 WASM 后端 live 未验证。
- **既有条目**：FR1–FR24、NFR1–NFR7 语义不变。

## 2026-10-22（第 20 轮，NFR6 v2 落地：TLS / 签名身份 / MCP HTTP / OIDC）

- **触发**：用户要求继续 NFR6 v2，并明确「MCP 通过 OIDC 从 IdP 获取身份」；三项能力（TLS 与签名身份、MCP Streamable HTTP、MCP OIDC）一次交付。
- **新增 FR22 TLS 与 Ed25519 请求签名身份**：可选 TLS 1.2/1.3（`[tls]`，`client_ca` 启用 mTLS）；`[auth.keys]` 非空即启用 Ed25519 请求签名，四头（participant/timestamp/nonce/signature）+ `PACT-SIGNATURE-V1` 规范化串，时间窗 ±300s、nonce TTL 300s/上限 8192、请求体 8 MiB 上限；签名与 Bearer 并存且签名优先；参与者不匹配 401，ACL 越权 403。
- **新增 FR23 MCP Streamable HTTP**：`[mcp].enabled = true` 时 daemon 内建 `/mcp`，工具面与 stdio 一致（17 个）；本地 tower 路由调用自身 REST，使用进程内能力令牌；`pact-mcp` 保留 stdio 薄封装；`allowed_hosts` 仅回环 + `public_url`。
- **新增 FR24 MCP OIDC 身份**：资源服务器模式，仅 `/mcp`；IdP 发现/JWKS 缓存、alg 白名单（RS256/ES256/EdDSA）、iss/aud/exp/nbf 校验、60s leeway、claim → participant 映射；`GET /.well-known/oauth-protected-resource` 与 401 `WWW-Authenticate` 发现链；IdP 不可达 503 且不影响 REST；令牌不缓存、不转发。
- **NFR5/NFR6 更新**：NFR5 增补两类 v2 身份；NFR6 标记 **v2 已实现（2026-10-22）** 并列出交付范围。
- **表述澄清**：启用 `[auth.keys]` 后 REST 非豁免请求必须签名，Bearer 不能替代签名；两者同时出现时以签名身份为准。
- **无删除既有需求**：FR1–FR21、NFR1–NFR7 编号与语义不变；§6 范围外未包含「MCP HTTP / TLS」，无需移除，其余排除项不变。
- **实现状态**：已实现（2026-10-22）；`cargo test --workspace` 219 项通过，TLS/mTLS/签名/重放/篡改/参与者不匹配、MCP Bearer 与 OIDC 正反向、元数据与 IdP 不可达均有集成覆盖（`crates/pact-http/tests/nfr6_v2.rs`）。

## 2026-10-21（第 19 轮，FR21 去除 WASM “实验性”标记）

- **FR21 状态变更**：WASM 本地分发形态由“实验性”转为**受支持**（D33，用户裁决）。稳定化前置 (1) Q7 CI、(2) 金标扩面、(3) `pact-engine-kernel` 抽取、(4) 宿主边界安全审视均已完成。
- **范围不变**：仍为 Node/桌面本地单用户嵌入；权威仍是原生 daemon；不提供 MCP/HTTP、ACL/审计、浏览器 OPFS/SharedWorker、DSH `govern` 与 NFR6 v2；不进入 `dist/` 产物集，不替代 `pactd`。
- **ABI 不变**：继续为 `pact-wasm/0.x` + JSON 字符串；去除标记不改变版本策略。
- **范围外表述同步**：§6 中“实验性 WASM 形态”改为“WASM 本地分发形态”，语义范围未变。

## 2026-10-21（第 18 轮，P6 决策门裁决 A：WASM 本地分发形态条件立项）

- **触发**：P0–P4 已完成（编译探针、纯类型层拆分、WASM 门面、内存引擎与原生对账、Node 持久化），P6 决策门评估（`docs/wasm-local-spike-feasibility.md` §11）建议 A；用户裁决 **A. 条件立项（实验性）**。
- **新增 FR21 WASM 本地分发形态（实验性，D31）**：WASM 为 `pactd` 之外的**可选本地分发形态**，权威仍是原生引擎，不替代 `pactd`；能力含语义核心、内存多实例会话、Rule-S/P/R/E/T、两阶段提交、JSONL/快照导出恢复与查询视图；持久化、崩溃恢复与单写者由宿主适配层承担（Node `crates/pact-wasm/js/` 已交付）。
- **能力边界**：不含浏览器 OPFS/SharedWorker、多用户 ACL/读审计、MCP/HTTP、TLS、turso 索引、压缩与运维接口；DSH `govern` 与 NFR6 v2 无硬依赖；安全边界为**本地单用户**，不得视为多用户安全边界。
- **ABI 策略**：`pact-wasm/0.x` + JSON 字符串；破坏性变更须升版并记入 `docs/design-change.md`；`abi_version` 为探测入口。
- **稳定化前置**（去除“实验性”标记的条件）：Q7 独立 CI job（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）→ 扩大金标（多实例、组合域、条件分支、压缩恢复）→ 抽取 `pact-engine-kernel` → 宿主边界安全审视。在此之前 WASM 不进入 FR18 生产分发承诺。
- **§6 范围外新增**：WASM 浏览器本地运行时、多用户 ACL/审计、DSH loop 治理与 NFR6 v2 远端模式；触发条件满足后另立范围。
- **既有条目**：FR1–FR20、NFR1–NFR7 语义不变；NFR4（单一静态二进制）仍是权威部署形态，FR21 不改变其表述。

## 2026-10-21（第 17 轮，阶段 6 CPN 编码生成转正式范围）

- **触发**：阶段 1–5、7–10 均已完成；用户在剩余候选（阶段 6 CPN / NFR6 v2 联网演进 / 加固收尾）中选定**阶段 6 pact-cpn**，将其从「范围外/可选」转为正式交付。
- **新增 FR20 CPN 编码生成**：新 crate `pact-cpn` + CLI 离线命令 `pact cpn <FILE> --out PATH [--instance I] [--bound T=N]…`；只依赖 `pact-core`，不进入 daemon/存储/引擎/运行时路径。
  - 单实例作用域（G1/D27）：一次生成只编码一个 `Counter instance`，库所/迁移以 `i_<instance>__` 命名空间化，不跨实例共享。
  - 结构映射：每类型库所 + Archive；`PACT_TOKEN = TokenId × Participant × ThingRef × PrecondRefs × Timestamp`；生产前驱 test arc、读自环、消费归档、种子无输入弧。
  - 条件：`and`/`or`/`not` → NNF → DNF；`or` 展开多迁移；`not` 下推为 `count T <= k` 并以互补库所编码（须显式 `--bound T=N` 真实上界）；`not via` 显式拒绝；恒假条件保留 `guard=false` 迁移；DNF 上限 1024。
  - 输出 CPN Tools XML（`format='6'`），通过 CPN DTD 校验与 XML 良构/结构测试；CPN Tools GUI 加载与状态空间实验为人工验收（沙箱无 GUI）。
- **§6 范围外收窄**：删除「CPN 编码生成器」整条，改为只排除「CPN 图形布局美化与 CPN Tools GUI 内交互式状态空间实验的自动化」；编码生成本身已属 FR20。
- **实现状态**：`pact-cpn` 11 项测试 + CLI 离线 e2e（成功/缺上界/JSON/实例前缀）；全量 **166 项**测试全绿；示例域生成物全部通过 CPN DTD 校验。
- 既有 FR1–FR19、NFR1–NFR7 无实质变更。

## 2026-10-20（第 16 轮，D25 子项 3 域级 ACL 与审计读视图落地）

- **触发**：用户裁决实施 D25 子项 (3)，四项取舍均选推荐项（实例级 ACL + 通配；覆盖生命周期 / 四类动作 / 查询；独立读审计流；fail-closed + REST 视图）。
- **FR17 细化（语义未变，仅把概括落为可实现口径）**：
  - ACL 条目 `domain/instance` 或 `domain/*`；非空时覆盖全部域作用域端点（生命周期、动作、查询、compact、审计视图），`GET /domains` 过滤；无令牌 `401`、越权 `403`；未配置 ACL 保持既有行为。
  - ACL 非空而令牌表为空 ⇒ daemon 拒绝启动；条目格式非法同样拒绝启动。
  - 读审计默认关闭；开启后每次成功 Rule-R 追加 `<instance>.read-audit.jsonl`（append + fsync），失败则本次读取 `500`（fail-closed）；审计流不进入事件日志 / `seq` / 重放 / Rule-T / 压缩。
  - 只读视图 `GET /domains/{d}[/instances/{i}]/read-audit?limit=`（缺省 100、上限 1000、时间正序）。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 编号与语义不变；NFR2 的「日志即完整审计」不因独立审计流改变（审计流是附加可选项，不替代事件日志）。

## 2026-10-20（第 15 轮，D25 子项 1 日志压缩与 schema 版本化落地）

- **触发**：用户裁决「开始实现日志压缩与 schema 版本化」，并就四项设计取舍选定推荐项：压缩触发 = **显式运维接口 + 可选启动自动**；截断机制 = **前缀删除 + 旁路 base_seq 清单**；幂等表 = **纳入快照**；version 形态 = **每事件顶层 `version` 字段**。
- **FR17 第 1 条细化（语义未变，仅把概括落为可实现口径）**：
  - 截断机制：快照为提交点 → `<instance>.logmeta.json` 记 `base_seq` 水位（先于日志重写）→ 日志 tmp+fsync+rename 原子重写（保留 `seq > base_seq` 行）；水位单调不回退。
  - 快照自足：快照承载 `version`/`seq`/`dl_seq`/`declaration`/`state`/`consumed_requests`，**Rule-E 幂等表随快照持久化**；恢复以快照为锚补尾部事件。
  - 数据安全：日志已截断而快照缺失/不覆盖水位 ⇒ fail-fast（`SnapshotRequiredForCompactedLog`）。
  - 版本兼容：`EVENT_SCHEMA_VERSION = 1`；缺 `version` 视为 v1；更高版本 fail-fast（`UnsupportedSchemaVersion`）。
  - 触发：REST / CLI / MCP 显式入口 + `pact.toml` 可选 `compact_on_start` / `compact_keep_last`。
  - 固有取舍：压缩后审计视图退化为「快照 + 保留尾部事件」，须在 design/manual 文档化。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 编号与语义不变；NFR2 的「日志即完整审计」在压缩后由「快照 + 保留尾部」共同承担（已写入 FR17）。子项 (3) 域级 ACL 与审计读视图仍未实现。

## 2026-10-20（第 14 轮，D25 子项 2 运维接口落地）

- **触发**：用户裁决「继续开发」，并选定阶段 8 先做**子项 (2) 运维接口**（另三项取舍：Prometheus 文本格式、运维端点免令牌、PID 文件可选+陈旧检测、系统化优雅停机）。
- **NFR7 条目具体化**（语义未变，仅把原先一句概括落为可实现口径）：`GET /health` / `GET /ready` / `GET /metrics`（Prometheus 文本）、可选 `pid_file`（缺省不写、含陈旧检测）、SIGINT/SIGTERM 优雅停机（停止接受新连接 → 有界等待在途 → 终态快照 → 清理）；明确**运维端点不校验令牌**（仅本地/内网）。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 编号与语义不变；子项 (1) 压缩与 schema 版本化、(3) 域级 ACL 与审计读视图仍未实现。

## 2026-10-20（第 13 轮，术语精化：幂等/自足作用域改用「域实例」）

- **触发**：用户指出第 12 轮「同域→同一实例」的措辞会与工程上的「实例」混淆，且须为将来类型-实例分离（如分布式 counter）留余地，要求统一写「域实例」。
- **措辞精化**：第 12 轮 G2 的「同一实例」→「**同一域实例**」，覆盖 FR9（事件自足）、FR10（Rule-T 信息性）、FR12、FR15、NFR2。术语映射见 `docs/design.md` D27（中文「域实例」= 论文 `Counter instance`，与论文已废弃的英文 `domain instance` 无关）。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 语义不变（仅中文措辞精化）。

## 2026-10-20（第 12 轮，多实例约束覆盖核对后的文档补齐 G1/G2）

- **触发**：用户要求核对「论文多实例并发约束的实现覆盖」。核对基准 = rev3 与多实例直接相关的条款（§1.3 两半、§1.4.1 唯一性、Rule-T scope note、§4.5/§B.2 CPN scope、§1.8 及 §4.1–4.3 术语）。核对结论：C1（按实例成立 / 跨实例不协同）、C2（TokenId 作用域 + 可加强）、C3（按实例终止）、C6（术语）**全覆盖**；**C4/C5（CPN 作用域）未登记（G1）**、**幂等作用域措辞滞后（G2）**。用户裁定「G1、G2 补文档；G3（母本 §1.4.1 Rationale 措辞不一致）特批改论文但不产生修订版本」。
- **G1（补登记）**：把论文 §4.5/§B.2 的 **CPN 单实例作用域**登记为阶段 6（`pact-cpn`，未开始）的**实现约束**。见 §6 范围外 CPN 条目。要求：每个 Counter 实例独立编码，库所与迁移不跨实例共享，跨实例协同不入 `N(D)`；CPN 编码的「域」为域声明，实例划分属运行期（D27）。
- **G2（措辞收窄）**：幂等/自足作用域由「同域」统一改为「**同一实例**」——D27 将「以域名为唯一会话键」收窄为「以实例为会话键」（FR19），代码先已按每实例实现，仅需求/设计文本滞后。涉及 FR9（事件自足）、FR12、FR15、NFR2。
- **无新增/删除需求**：FR1–FR19、NFR1–NFR7 语义不变（仅措辞随 D27 收窄）。CPN 约束为**既有范围外项的实现前置约束**，不改变其「后期可选阶段」定位。

## 2026-10-20（第 11 轮，阶段 10 实施：FR19 落地与细化）

- **触发**：用户「按计划实施」——实施阶段 10 域双键实例化（D27，路径 β，`docs/design.md` §11 行 10）。**本轮仅 D27 双键本体**，阶段 8 仍暂停。
- **FR19 细化（实施前置的四项裁决 + 通用护栏）**：
  - **默认回退**：不指定实例 ⇒ `default` 实例；既有 `--domain` 调用与脚本语义不变（落到 `default`）。
  - **旧日志不兼容（用户裁决）**：双键布局只识别 `<data_dir>/<domain>/<instance>.events.jsonl`；顶层遗留单层文件（`<domain>.events.jsonl` / `<domain>.snapshot.json`）**不再被识别**，启动恢复 **fail-fast**（`StoreError::LegacyLayoutDetected`），须人工迁移，不静默丢数据。
  - **实例键承载形态（用户裁决）**：实例名作为**路径段**（`/domains/{d}/instances/{i}/…`），`/domains/{d}` ≡ `default` 实例；`DomainLoaded` **不新增 `instance` 字段**（实例由文件路径承载）。
  - **声明原文另存**：`declare` 时写 `<data_dir>/<domain>/declaration.pact`，`recover` 用它回填声明注册表（支持重启后新增实例）。
  - **通用护栏（用户裁决）**：同一 `data_dir` 上 daemon 启动须取独占 flock（`<data_dir>/pactd.lock`），第二实例启动即拒；对**非空实例**（`last_seq > DomainLoaded.seq`）再加载默认拒绝（`DomainBusy`），须显式 `force`。
- **FR19 实现状态行**：由「立项（未实现）」更新为「**已实现（阶段 10，路径 β）**」——存储 `(domain, instance)` 双列 + 域子目录；引擎声明注册表（`declare` / `instantiate` 分离）；接口全链（REST 默认族 + 实例镜像族、CLI `--instance`/`--force`/`instantiate`/`instances`、MCP 16 工具含 `pact_domain_instantiate`、`pact-runtime` 实例透传）；键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。
- **FR10 / NFR7 无删改**：多域会话行与多实例边界行维持第 10 轮表述（FR19 已涵盖实例维度）。
- **FR17 维持暂停**：引擎生产化三项仍与实例化耦合，未在本轮实现。
- **无其他需求增删**：FR1–FR18、NFR1–NFR7 语义不变。

## 2026-10-20（第 10 轮，域双键实例化：FR19 + FR10/NFR7 补充，论文 rev2 → rev3）

- **触发**：用户提问「两个同一个域的项目并行如何处置？」→ 指示暂停阶段 8 并评估模型 C 的全面影响。领域专家裁定（要点见 `docs/design-change.md` 第 18 轮）后，用户裁决**立项**、实现走**路径 β**、并破例授权把论文修订为 rev3。
- **范式来源切换**：`docs/pactv1_rev2.md` → `docs/pactv1_rev3.md`（rev3 仅含术语与作用域增补：§1.3 增列「Counter instantiation and partitioning」、§1.4.1 TokenId 作用域、Rule-T scope note、§4.5/§B.2 CPN 作用域、`domain instance` → `domain declaration`）——**零文法变化、零语义变化**，故 FR4–FR9 语义条目无需重述。
- **新增 FR19 域双键实例化**：定义键 `domain` 与实例键 `instance` 分离；同一域声明可并发承载多个 **Counter 实例**，各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / Rule-E 幂等表；实例为接口层一等键（REST 路由、CLI、MCP、ACL、审计、压缩、运维指标）；不指定实例 ⇒ 默认实例，既有调用与旧日志语义不变。归属 `pact-store` + `pact-engine` + 接口层；`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。
- **FR10 补充**：多域会话行增「域内可并发承载多个 Counter 实例」。
- **NFR7 补充（多实例边界）**：同一 `data_dir` 并发承载多实例须以独占锁（flock）保证单写者，禁止交错写同一日志；实例间不共享 Counter/Archive，跨实例协同不在范式保障范围内。
- **FR17 无删改**：引擎生产化三项维持有效，但验收与实例化耦合——(1) 的压缩部分与 (3) 全部须按双键重述，`:compact` 路由须随实例段重设计。
- **无删改**：FR1–FR16、FR18、NFR1–NFR6 语义不变。

## 2026-10-19（第 9 轮，生产化边界裁决：FR16–FR18 + NFR7）

- **触发**：用户就"作为生产系统的功能边界"提问——还需补足什么、留在本项目还是另立、同一二进制还是分开构建、MCP server 由谁提供。经四问裁决：(1) agent 驱动运行时 → 本仓库新增 crate；(2) 引擎生产化三项全部推进；(3) 分发产物 → 本仓库 `dist/`；(4) MCP 维持 stdio-only 到 v2。
- **新增 FR16 生产运行层**：可复用的参与者驱动运行时（自驱循环 + 多 agent 编排）+ 外部副作用执行器；归属本仓库新增 crate（决策 D24）。引擎保持被动（FR11/D5）、核心语义零改动。
- **新增 FR17 引擎生产化**：事件日志压缩与 schema 版本化（pact-store）、运维接口（health/ready、metrics、PID、信号化优雅停机）、域级授权与审计读视图（读审计可选开启、默认关闭，修订 D9 默认值表述）。
- **新增 FR18 部署与分发产物**：本仓库 `dist/`（Dockerfile、systemd unit、musl 静态二进制打包脚本），属交付资产，不接入 CI 语义测试（同 D22）。
- **新增 NFR7 可运维性**：可被外部进程管理器托管（可探活、可优雅停机）、运行状态可观测、数据可备份恢复、事件日志长期增长有可控压缩/保留手段。
- **无删改**：FR1–FR15、NFR1–NFR6 语义不变；§6 范围外三条维持（CPN 生成器、Archival Policy、分布式/OR/时间/概率）；原"引擎不执行外部副作用"边界不变——副作用执行端移入 FR16。

## 2026-10-19（第 8 轮，开发环境描述更正）

- **触发**：用户指出开发环境描述过时——`pact-dev` 容器已不存在，现开发环境为 Trae 沙箱环境。
- **§2 总体约束「开发环境」行更正**："所有开发工作在 distrobox 容器 `pact-dev` 中进行" → "所有开发工作在 Trae 沙箱环境中进行"；同步更正 `AGENTS.md` 同名规则条与 `docs/skill-adaptation.md` §9.1/§9.2 的环境表述（技能侧记录见 `docs/skill-adaptation-change.md` 第 5 轮）。
- **无功能需求变更**：FR1–FR15、NFR1–NFR6 均不变，仅环境约束的文字表述更新。

## 2026-10-18（第 7 轮，消耗原子性、幂等与物化边界）

- **触发**：对"消耗原子性"组织评审后，用户裁决采纳"文档化实现契约 + 补幂等 + witness-token 示例（2a）与 outbox 语义（2b）并行"组合，要求契约表述自足、不依赖外部讨论语境。
- **新增 FR15 消耗原子性、幂等与物化边界**：明确引擎对 Rule-E（消费）在并发、崩溃、重试与外部副作用四个维度上的承诺与不作为——提交点（JSONL fsync）前无部分效果、提交点后派生步骤尽力而为、Rule-E 支持可选幂等键、引擎不执行外部副作用。
- **FR9 补两条**：事件自足（Consumed 事件携带 token 全量快照，单行即可审计与重放）、提交契约（JSONL fsync 为提交点，其后派生动作不回滚）。
- **FR10 补一条外部副作用边界**：`ThingRef` 不透明、引擎不执行外部动作，"consume 成功" 不等于 "外部动作成功"，物化确认由 witness-token 或 saga 语义在域外承载。
- **FR12 补一条**：Rule-E 消费请求支持可选 `request_id` 幂等键；同键同 token 重试返回首次归档结果，同键异 token 报 `IdempotencyConflict`（HTTP 409）。
- **FR13 集成测试补幂等**：新增 request_id 去重、跨重启幂等表重建、同键异 token 冲突三类断言。
- **FR14 域数四 → 五**：通用域新增 `external_materialization.pact`（witness-token 物化确认模式）。
- **NFR2 强化**："每条事件自足"——单行日志即可独立审计与重放，不依赖同域其他行。
- **§6 范围外新增一条**：分布式事务 / 2PC / 分布式共识、以及由引擎执行外部副作用，均不在实现范围内（单写者 actor 已满足线性化需求，见 design.md D23）。

## 2026-10-15（第 6 轮，通用示例与用户手册）

- **触发**：用户指出既有 `docs/skill-adaptation.md` 一族的适配结构与 requirements-engineering / software-engineering 两技能强绑定，需补一套与技能无关的需求工程/软件工程通用结构，并配 examples 向用户说明整个 PACT 范式系统的使用场景与方法。
- **新增 FR14 通用示例与用户手册**：`examples/domains/` 四份通用域、`examples/fragments/` 两份组合片段、`examples/offline/run.sh` 与 `examples/online/run.sh` 两条上手路径（各带录制轨迹）、`examples/README.md` 场景导航；`docs/manual.md` 为权威用户手册。明确二者分工（可运行教学路径 / 参考权威）与交叉引用义务。
- **§6 范围外新增一条**：示例不接入 CI 回归——`examples/**/*.pact` 不由 cargo test 遍历校验，示例正确性由人工执行与录制轨迹对照保证（决策 D22）。
- **用户裁定的范围边界**：通用域含 RE / SE / ReAct-agent 三类，**不含**医疗路径域；SE 通用域取中等粒度；examples 深度为离线三件套 + 在线 daemon 全生命周期轨迹；用户向文档落点为 `examples/README.md` 与 `docs/manual.md` 两者。
- 既有 FR1–FR13、NFR1–NFR6 无实质变更。

## 2026-09-09（第 5 轮，论文 rev2 对齐）

- 范式规范来源切换：`pactv1_rev1.md` → `pactv1_rev2.md`。
- FR4：Rule-P 前驱互异补充"只按 TokenId 判定"（rev2 Rule-P Note）。
- FR5：新增 count 字面量 PosNat 约束（`[1-9][0-9]*`，拒绝 0 与前导零；缺席语义用 `not exists T` 表达，Normalization rule）。
- FR6：良构校验由"四条零散规则"重述为 §3.1 六项约束（新增 LHS Uniqueness、Non-Empty LHS 两条；自环 `A → A` 显式合法）；明确解析期语法拒绝 + 声明期 validate_domain 防御两层。
- FR7：A.5 由"源生产合法性"重定义为"前置与源生产良构性"（rev2 A.5）——§3.1 六项约束违规清单 + Seed/Produce 重叠咨询性警告，三元输出 Violations/Warnings/WF(D)，按附录 D 为其余分析的前提、置于报告首位。

## 2026-08-26（第 4 轮）

- 范式规范来源切换：`pactvfinal.md` → `pactv1_rev1.md`（论文最新版解决了 D15 指出的 Rule-T 终止问题，以 rev1 为准）。
- FR4：Rule-P 补充前提 `Precond(T) ≠ ∅`（源类型不可生产）；Rule-T 按 rev1 Rule-P-enabled 三条件定义。
- FR6：良构校验补充 WF-3——源类型（无前置规则）不得声明 produce gate。
- FR7：静态分析由五项扩为六项（新增 rev1 A.5 源生产合法性）。
- FR8：片段组合一致性由三条件扩为四条件（新增 C4 源生产封闭）。

## 2026-08-25（第 2 轮）

- 文档标题去掉"（当前有效）"后缀（三文档统一）。
- 新增 FR12 领域数据格式：`.pact` 声明文件为注入点唯一载体；定义 Token/ArchiveEntry/动作请求响应/结构化错误/通知事件的 JSON 线上格式；三接口共享同一套格式；时间戳 ISO-8601 UTC、字段 snake_case。
- 新增 FR13 测试组织：随变更组织单元/集成/端到端/回归四类测试（对应 AGENTS.md 新增规则）。

## 2026-08-25（第 1 轮）

- 初始需求建档（FR1–FR11，NFR1–NFR6，范围外清单）。
