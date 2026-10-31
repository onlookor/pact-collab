# 架构变更记录

> 历史记录保留当时的编号与措辞；现行编号约定见附录 A。

## 2026-10-31（第 46 轮，参考平面 resolve/describe 端点契约与鉴权定案）

- 参考平面（治理平面）首版接口收敛为 `POST /v1/resolve`、`POST /v1/describe` 与不带版本的 `/healthz`、`/readyz`；invoke/revoke 延后到动态阶段。规范产物为 `spec/ref/v1/reference-plane.openapi.yaml`，设计定案在 `docs/thingref-governance.md` §13.6。
- 边界不变：`pactd` 永不调用 `refd`，`refd` 永不直接写 PACT 状态；PACT token read gate 与制品 ACL 为两层独立权限，参与者必须同时满足。
- 鉴权方向：主体由验证后的凭据（OIDC JWT 或静态令牌）派生，请求体不接受 `actor`；端点 scope 与对象 ACL 并集允许、默认拒绝；每条已认证请求一条审计事件，审计失败 fail-closed。
- 数据流不变：`refd` 只返回不透明 `handle`/摘要/媒体类型/契约标识与时效，不返回制品正文；证据仍走 witness token。
- 跨仓库：`pact-ref/docs/architecture.md` 的 “Reference Plane API” 与“引导阶段下一步”已同步指向冻结契约；`pact-ref` 代码与 vendored 快照未改动。
- 文档：`docs/design-change.md` 第 90 轮、`spec/ref/v1/reference-plane.openapi.yaml`、`docs/thingref-governance.md` §9.2 / §13.1 / §13.6、`todo.md`。

## 2026-10-31（第 45 轮，WASM 本地入口校验，D50）

- 附录 A 第 88 轮登记 `D50`（WASM 本地入口票面校验与绑定注入），权威定义在 `docs/design.md` §0 / §9.17。
- 架构正文“票面写入校验在接口层”更新：`pact-wasm` 本地宿主通过宿主注入的 RefBindings 复用同一 `pact-ref-spec::validate_entry` 纯函数（FR30/D50），未注入绑定时只做结构校验；ContractCard 归档钉版仍只在 REST 边界。
- 组件与依赖方向：`pact-ref-spec` 仍是纯规范层（只依赖 `serde`，可编译 wasm32，无 `pact-core` 依赖）；`pact-wasm` 新增对它的依赖，`pact-http` 删除本地入口语义后继续委托同一实现。引擎、存储与事件日志仍不感知票面。
- 数据流不变：WASM 仍在宿主持久化后提交；绑定注入只影响本进程内存，宿主重启需重新注入。
- 文档：`docs/requirements.md` FR21/FR30/FR31、`docs/design.md` §0/§9.12/§9.17、`docs/manual.md` §2.7/§7.11、`docs/thingref-governance.md` §8.1/§8.2/§13.1、README 中英文本地嵌入描述。

## 2026-10-31（第 44 轮，`[ref_digest]` 与 `DeclarationDigestMismatch` 登记与运行期落点）

- 附录 A 登记运行期配置项 `[ref_digest]` / `[ref_digest].mode` 与 HTTP 错误码 `DeclarationDigestMismatch`（均无字母+数字编号，仍先登记后使用）。
- 架构正文“票面写入校验在接口层”更新：D49 第二步已实现，`pact-http` 在 `POST /domains` 进入引擎前按 `[ref_digest].mode` 比对 `declarationDigest`；比较只读 `pact-analysis` 的纯函数与当前绑定快照，引擎、存储与事件日志仍不感知摘要。
- 组件与依赖方向不变：`pact-http` 既有 `pact-analysis`/`pact-parser` 依赖，未新增 crate 或依赖；未改变 `pact-ref-spec` 的零 `pact-core` 依赖。启动恢复路径与 D48 重载路径不进入比较，数据流不变。
- 文档：`docs/manual.md` §2.2 / §7.14 / §8.2 / §9.4 / §10、`docs/design.md` §9.16、`docs/thingref-governance.md` §8.1 / §13.1 / §13.4.2、`dist/pact.container.toml` 同步。

## 2026-10-31（第 43 轮，`declarationDigest` v1 第一步）

- 附录 A 登记 `FR34` / `D49`（域声明语义接口摘要 v1），权威定义在 `docs/requirements.md` §4 与 `docs/design.md` §0/§9.16。
- 架构正文“票面写入校验在接口层”补充：摘要的规范化与计算只存在于离线工具 `pact ref digest` 与 `pact ref lint`，实现由 `pact-analysis` 提供；运行期比对尚未启用，引擎、存储与事件日志不感知摘要。
- 组件与依赖方向不变：`pact-analysis` 新增 `digest` 模块（依赖 workspace 既有 `serde_json`/`sha2`），`pact-cli → pact-analysis` 已在依赖图中；未新增组件、未改变 `pact-ref-spec` 的零 `pact-core` 依赖。
- 规范资产新增 `spec/ref/v1/declaration-digest.md`，`ref-bindings.schema.json` 的 `declarationDigest` 增加指向该规范的 `description`；schema 结构与 `pattern` 不变。

## 2026-10-30（第 42 轮，B1 显式重载实现）

- 架构正文“票面写入校验在接口层”更新：卡片/绑定显式重载（FR33/D48）已实现，经控制面 `POST /ref/reload` 原子替换 `pact-http` 的规则快照；不进入引擎、存储或事件日志。
- 新增 `pact-http::reload` 模块（配置、审计、影响预检）；`ServerState` 的规则快照改为可原子替换的 `RwLock<Arc<VisibilityProfiles>>`，请求级持同一快照。依赖方向不变（`pact-http → pact-ref-spec`），未新增组件或依赖。
- 审计落点 `<data_dir>/ref-reload.jsonl` 属控制面旁路文件，不进入 `seq`/重放；引擎与 `pact-core` 仍不感知票面与卡片。
- 文档：`docs/design.md` §9.15 与 D48 状态、`docs/manual.md` §7.12/§8/§9、治理 §13.5 同步。

## 2026-10-30（第 41 轮，B0 契约卡加载实现）

- 架构正文“票面写入校验在接口层”更新：ContractCard 运行期加载与版本不可变（FR32/D47）已实现，仍在 `pact-http` 接口层；卡片/绑定显式重载（FR33/D48）尚未实现。
- daemon 启动期新增 `[ref_contracts].paths` 的加载与引用闭环校验；卡片只用于 REST 边界的归档钉版并集，引擎、存储与 `pact-core` 仍不读取卡片，依赖方向与数据流不变。
- 未新增组件或依赖；`pact-ref-spec` 仍是唯一卡片解析/校验实现。

## 2026-10-29（第 40 轮，ContractCard 加载与重载编号登记）

- 附录 A 登记 `FR32` / `D47`（ContractCard 运行期加载与版本不可变，B0）与 `FR33` / `D48`（ContractCard 与 RefBindings 显式重载，B1），权威定义分别在 `docs/requirements.md` §4 与 `docs/design.md` §0。
- 架构正文“票面写入校验在接口层”补充：卡片加载与重载已定案、尚未实现，实现后仍位于 `pact-http` 接口层；引擎、存储与 `pact-core` 不读取卡片。
- 未新增组件、依赖方向或数据流；`pact-ref-spec` 仍是唯一卡片校验实现，B1 重载校验须与其共用。

## 2026-10-29（第 39 轮，形态/未知 scheme/钉版运行期强制）

- **接口层扩展**：在既有写入校验（FR30/D45）之上增加 `value`、`onUnknownScheme` 与 `pinning: required`（写入）/`required-at-archive`（consume 前）判定（FR31/D46）；consume 检查从 `engine.counter` 读取待消费 token 的票面，引擎与存储仍不解析 `thing_ref`。
- **依赖方向不变**：`pact-http → pact-ref-spec`；`pact-core`/`pact-engine`/`pact-store` 不新增治理依赖。消费路径只多一次内部视图读取，不改变事件、快照与 Rule-P/GateExpr 语义。
- **入口边界不变**：CLI/MCP/`pact-runtime` 经 REST 继承；`pact-wasm` 本地宿主仍不在边界内。
- **文档**：附录 A 登记 `FR31`/`D46`；`docs/requirements.md` 新增 FR31；`docs/design.md` 新增 D46 与 §9.13；`docs/manual.md` §7.11 补充形态、未知 scheme、钉版与错误码。

## 2026-10-28（第 38 轮，ThingRef 写入校验）

- **接口层新增写入边界**：`pact-http` 在 `POST /seeds` 与 `POST /tokens` 调引擎前执行票面三态判定、URI profile 校验与 RefBindings 策略（FR30/D45）；引擎调用之后票面仍为不透明 `ThingRef`。
- **依赖方向不变**：`pact-http → pact-ref-spec` 已存在（第 35 轮视图过滤引入）；`pact-core`、`pact-engine`、`pact-store` 仍不依赖治理 crate，GateExpr/Rule-P/Rule-T 不读取票面。
- **入口继承**：CLI 与 MCP 经 REST client 自动继承校验；`pact-wasm` 本地宿主直接构造 `ThingRef`，不在该边界内（登记于 `todo.md`）。
- **文档**：`docs/requirements.md` 新增 FR30；`docs/design.md` 新增 D45 与 §9.12；`docs/manual.md` 补充 REST 规则与错误码；`docs/thingref-governance.md` 更新 §8.1/§13.1 状态；新建 `todo.md` 进度台账。

## 2026-10-27（第 37 轮，引用保留与 tombstone）

- **规范层职责扩展**：`pact-ref-spec` 在 URI/RefBindings/ContractCard 之上新增 Tombstone 与 lifecycle 校验；仍为纯 crate，不依赖引擎、存储或 IO。
- **静态验收路径**：`pact-cli` 的离线 `ref lint --tombstones` 消费规范层，校验 ContractCard 保留策略、Tombstone 字段与二者一致性；不经 daemon。
- **边界不变**：PACT 引擎不 fetch/delete/解析引用，tombstone 由域作者以正常 seed/produce 产生；保留计时、内容删除与 tombstone 发布属于治理平面/制品域。
- **文档**：`docs/requirements.md` 新增 FR29 并扩展 FR27；`docs/design.md` 新增 D44 与 §9.11；`docs/manual.md` 新增 §7.10 并更新 §7.8；`docs/thingref-governance.md` 更新 §9/§13 阶段状态与产物清单。

## 2026-10-27（第 36 轮，视图级读审计）

- **审计流扩展**：独立读审计流新增 `kind=view` 记录（D43），承载 Counter/Archive 的按类型暴露计数；Rule-R 记录保持 `kind=read`，旧记录缺省兼容。
- **写路径归属**：视图计数在 `pact-http` 过滤票面时计算，落盘仍经引擎 actor 单写者（`Engine::record_view_audit`），接口层不直接写文件。
- **安全边界**：审计只记计数、不记 `thing_ref` 值；写失败时视图 `500` 拒绝。审计流仍不进入事件日志、`seq`、重放与 Rule-T。
- **依赖方向**：`pact-http → pact-store` 新增 `ViewName`/`ViewAuditType` 类型引用；`pact-core`、`pact-engine-kernel` 仍不感知引用与审计。
- **文档**：`docs/requirements.md` 扩展 FR17/FR28；`docs/design.md` 新增 D43 与 §9.10 记录示例；`docs/manual.md` §7.5/§7.9 更新；`docs/thingref-governance.md` §9/§13 更新阶段状态。

## 2026-10-27（第 35 轮，物引可见性过滤）

- **接口层新增策略**：`pact-http` 在 Counter/Archive 返回前过滤 `thing_ref`（D42/FR28）；默认 `read-gate`，`domain` 公开，`hidden` 隐藏；token 存在性与事件日志不变。
- **依赖方向**：`pact-http → pact-ref-spec`（新增 workspace 依赖）与 `serde_yaml`；引擎、内核、存储仍不解析引用。
- **配置与启动**：新增 `[ref_bindings].paths`；路径展开、解析、规范校验与重复域检查在 daemon 启动时完成，失败 fail-closed。
- **身份边界**：令牌/签名模式下视图读取要求认证；信任模式无主体，read-gate 默认隐藏。
- **文档**：`docs/requirements.md` 新增 FR28；`docs/design.md` 新增 D42；`docs/manual.md` §2.2/§7.9 增补配置与行为；`docs/thingref-governance.md` §9/§13 更新阶段状态。

## 2026-10-27（第 34 轮，物引静态检查命令）

- **新增离线命令**：`pact ref lint`（`crates/pact-cli/src/ref_lint.rs`），只读文件、只依赖 `pact-ref-spec` 与纯函数层，不经 daemon、不触状态。
- **依赖方向**：`pact-cli → pact-ref-spec`（新增 workspace 依赖）与 `serde_yaml`；规范 crate 仍不反向依赖引擎/store。
- **归属**：属范式引擎层之外的离线工具，不进入 daemon 拓扑，不改变存储、进程与端口。
- **文档**：`docs/requirements.md` 新增 FR27；`docs/design.md` 新增 D41；`docs/manual.md` §7.8 与 §8.1 增补命令说明；`docs/thingref-governance.md` §9/§13 更新落地状态。

## 2026-10-26（第 33 轮，物引规范层）

- **新增 crate**：`crates/pact-ref-spec`，纯规范层，不依赖 engine/store，可编译到 wasm32。
- **新增规范目录**：`spec/ref/v1/`（URI profile、两份 JSON Schema、Reference Plane OpenAPI）。
- **依赖方向**：`pact-ref` → `pact-ref-spec`（re-export + vendored 快照）；`pact-ref-spec` 不反向依赖 PACT 引擎。PACT daemon 不调用治理平面。
- **文档**：`docs/architecture.md` §1 新增“物引规范层”约束；`docs/design.md` 新增 D40；`docs/thingref-governance.md` §9/§13 同步落地状态。

## 2026-10-25（第 32 轮，文档编号约定附录）

- **新增附录 A「文档标识符与编号约定」**：登记全部命名空间的权威定义文件（FR/NFR、D、TC-*、B、W-*、契约 G/H/R、E、S、Rule-*、A.x）与跨文件引用规则（先登记后使用、跨文件带前缀、不复用含义、描述性概念不用标签、历史不回溯、测试夹具除外）。
- **架构分层去标签**：§1/§6/§7 中的 `L1/L2/L3` 改为描述性名称（范式引擎层 / 领域适配层 / 生产运行层 / 宿主侧控制层）；DSH 控制层原本表述为“L1/L2/L3 之外”，一并改写。
- **冲突消除**：`R1–R6`（契约配方）与 WASM 风险 `R1–R10` 通过 `W-R*` 前缀区分；`S0–S3`（WASM 宿主阶段）通过 `W-S*` 区分于 skill 工作流 `S0–S7`。
- **不变项**：拓扑、依赖方向、进程与端口、存储布局与存储格式均不变。

## 2026-10-25（第 31 轮，控制面认证与审计）

- **边界归属**：控制面认证/授权仍位于接口层（`pact-http`），审计字段由存储层控制面文件承载，引擎只透传已认证身份；`pact-core` 零改动，无新进程、端口或依赖。
- **授权语义**：ACL 启用时，交接因改写域级 `declaration.pact`，同时校验源实例键、目标实例键与 `domain/*` 通配键；活动切换要求 `domain/*`；冻结与活动查询按实例键。配置任一身份后，`handoff`/`freeze`/`active` 必须可认证；既有生命周期端点保持信任行为。
- **可观测性**：`.sealed` 与 `active.json` 记录 `ts`/`by`，`GET /active` 暴露二者；未引入外部审计系统或跨实例协同。
- **不变项**：单权威 `Engine` actor、单写者、`dist/` 产物与 WASM 能力边界不变。

## 2026-10-25（第 30 轮，D39 显式状态交接与活动实例别名）

- **存储视图**：域目录新增 `active.json`（活动实例指针）与 `<instance>.sealed`（冻结标记）；二者均为小体积控制面文件，不进入事件日志，不参与 `seq`、快照与压缩。
- **运行时视图**：短路由 `/domains/{d}/…` 先解析活动指针再定位实例会话；实例路由 `/domains/{d}/instances/{i}/…` 不解析指针。交接在引擎临界区内完成「冻结 → 追加 `Imported` → 建会话」，源实例此后只读，目标以 `Imported` 自足恢复。
- **质量属性**：换版期间源实例状态保留且可读，失败路径无副作用；活动指针悬空在启动期 fail-fast，避免静默误路由。单权威、单写者与 `dist/` 产物不变。
- **无结构性新增**：无新进程、无新端口、无跨实例协同；WASM 形态仅补 `DomainListing.active = None`，不提供交接能力。

## 2026-10-23（第 29 轮，DSH 插件独立为外部仓库）

- 宿主集成层由“本仓库内 `plugins/dsh/` 目录”改为“独立仓库 `pact-dsh`（包名 `dsh-plugin-pact`）”；依赖方向仍为单向 `pact-dsh → DSH SDK / PactBackend`，本仓库 `crates/` 不依赖插件。
- `crates/pact-wasm/js/open-host.js` 保留在本仓库，作为 WASM 宿主组装入口；插件以配置中的绝对路径动态 import，二者不构成构建期依赖。
- 本仓库目录结构不再包含 `plugins/`；`dist/`、Cargo workspace 与 CI 的 Rust/wasm job 均不受影响。
- 文档层面 `docs/architecture.md` §1/§7 的归属与依赖描述已同步；历史记录保留迁移前的拓扑。

## 2026-10-23（第 28 轮，D37 WASM 后端接缝落地）

- **接缝**：新增 `crates/pact-wasm/js/open-host.js`（CommonJS）作为“WASM 产物 + `FileStore` + 声明 → `PactHost`”的组装入口；DSH 插件以配置中的文件路径动态 import，不形成 ESM 包依赖，`crates/` 仍不被 `plugins/` 依赖。
- **§7 更新**：新增“WASM 后端接缝”条目，登记 `backend=wasm` 下三条路径的实跑结果与实例日志/锁由 DSH 进程内 WASM 宿主创建。
- **无结构性变更**：进程内拓扑、依赖方向、权威边界、单写者与 `dist/` 产物均不变；插件不新增端口、不拉起子进程。

## 2026-10-23（第 27 轮，D37 真实模型门控复跑补充）

- **验证补充**：以 `deepseek-official/deepseek-flash` 在 headless profile 中复跑 `pact-demo` preset 的真实 agent turn，允许、`denyTools` 工具拒绝、`fakeEnabled=false` 预检拒绝三条路径均按设计生效。
- **§7 更新**：“装配与门控验证”条目补入真实模型供应商复跑，确认 `pact-*` preset → `PactAgentLoop` → agent scope 钩子 → 模型/tool 的完整控制流。
- **无结构性变更**：进程内拓扑、依赖方向、权威边界、单写者与 `dist/` 产物均不变；凭据未进入架构或仓库。

## 2026-10-22（第 26 轮，D37 真实会话门控验证补充）

- **验证补充**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 的 headless profile 中以 `pact-demo` preset 与本地确定性 mock LLM 驱动真实 agent turn，验证 `govern` 允许、`denyTools` 工具拒绝、`fakeEnabled=false` 预检拒绝三条路径均按设计生效。
- **§7 更新**：“装配验证”条目扩为“装配与门控验证”，登记真实 `ctx.agents.create` → `PactAgentLoop` → agent scope 钩子 → 模型/tool 的完整路径。
- **无结构性变更**：进程内拓扑、依赖方向、权威边界、单写者与 `dist/` 产物均不变；插件不新增端口或子进程。

## 2026-10-22（第 25 轮，D37 真实 DSH 装配验证）

- **装配事实**：在 `@deepseek-ai/dsh@0.2.1-rc.2` 的 web profile 中安装 `plugins/dsh/` 并启动；`--dump-config` 显示原 `agent-loop` 行 `disabled: true`，`pact-agent-loop` 与 `dsh-pact-observe` 插入；运行时 `ctx.agentLoop` 为 `PactAgentLoop`，继承官方 `AgentLoop`。
- **§7 图注**：`agent-loop 行（被 cordis.patch.yml 覆盖）` 修正为 `agent-loop（原行 disabled，pact-agent-loop 接管）`；新增“装配验证”条目。
- **依赖方向**：`@deepseek-ai/dsh-agent-loop` 明确为 peer，由 DSH 安装解析；拓扑、进程/端口、权威边界与 `dist/` 产物不变。
- **未变**：crate 拓扑与依赖边、单一状态变更通道（D6/D19）、引擎被动性（D5）、权威边界（原生 daemon 唯一权威）。

## 2026-10-22（第 24 轮，DSH 宿主集成层：`plugins/dsh/`）

- **新增宿主侧控制层**：`plugins/dsh/`（`@pact/dsh-plugin`，非 Cargo）与 `crates/` 并列，作为 DSH 进程内的控制流适配器；不在原 L1/L2/L3 分层内，不改变 daemon、引擎、存储与 `dist/` 拓扑。
- **依赖方向**：`plugins/dsh → DSH SDK（agent-loop/agent/tools/session-projection）+ PactBackend`；不被任何 Rust crate 依赖；不新增监听端口、不拉起子进程。
- **组件角色**：`PactAgentLoop extends AgentLoop` 接管 `AgentFactory` 槽位，仅对 `pact-*` preset 安装 agent 级钩子；独立观察行镜像 `session/event`；门判定委托 `FakePactBackend`/`WasmPactBackend`，插件不持有语义状态。
- **§1 总览**：新增“宿主集成层（非 Cargo，阶段 11）”约束。
- **新增 §7**：DSH 宿主集成视图（进程内拓扑、依赖方向、权威边界、失败方向、归属）。
- **未变**：crate 拓扑与依赖边、单一状态变更通道（D6/D19）、引擎被动性（D5）、权威边界（原生 daemon 唯一权威）、`dist/` 产物集。

## 2026-10-22（第 23 轮，NFR6 v2 架构落地）

- **§1 总览**：daemon 由「单机单进程」表述更新为「单权威进程，unix 默认 / TCP+TLS 可选」；接口图补 MCP Streamable HTTP 与身份层（Bearer/签名/OIDC）；新增「身份止于接口层」约束。
- **§2.1 数据流**：MCP 请求路径改为 `pact-mcp / 内建 /mcp → REST → engine`，并标注中间件身份解析（Bearer/Ed25519 签名；HTTP 侧 OIDC）。
- **§2.5 ACL 流**：身份来源改为 Bearer 或签名；豁免路径改列 `/health|/ready|/metrics|/mcp|RFC 9728 元数据`，说明 `/validate` 在签名启用时仍需签名。
- **新增 §2.7**：MCP Streamable HTTP 与 OIDC 数据流（`require_mcp_auth` → JWT 校验 → `PactMcp` → 进程内 tower 调 REST → engine；401/503 边界；令牌不缓存/不转发；信任模式限制）。
- **§3 进程与部署视图**：改为 v2 拓扑（TLS 终止在 pactd、内建 `/mcp`、外部 IdP/JWKS、stdio 薄封装保留）；明确 OIDC 为可选外部依赖且仅 `/mcp` 触发。
- **§4 联网拓扑**：由「v2 预留」改为「v2 已实现」；新增部署拓扑图（TLS in pactd、mTLS 可选、签名、内建 `/mcp`、IdP/JWKS）、请求身份判定顺序表、v1→v2 差异表；明确引擎/存储/核心零改动，新增仅 `pact-http` 的 `tls`/`sign`/`oidc`/`mcp` 模块。
- **§5 安全边界**：更新为 TLS/mTLS + Bearer/签名 + ACL；新增「联网身份与传输」行（OIDC 仅 `/mcp`、令牌不转发、IdP 故障 503、内置客户端不出示客户端证书）。
- **§6 生产运行层**：runtime 连接参数补 `--https/--tls-ca/--sign-key/--sign-as`；删除「服务端 MCP 随 v2 落地」「v1 无 TLS」等陈旧表述；分发仍要求默认仅绑回环/受信网络，跨主机时用 TLS/mTLS 或外部终结 + 令牌/签名/ACL。

## 2026-10-21（第 22 轮，D33：WASM 本地分发形态去除“实验性”标记）

- **组件状态**：`pact-wasm` 由“实验性本地分发形态”转为**受支持的本地分发形态**（D33）。组件职责、依赖边与运行边界不变：仍是进程内、本地单用户、Node/桌面嵌入；权威仍是原生 daemon。
- **边界不变**：不提供 MCP/HTTP、ACL/审计、浏览器 OPFS/SharedWorker、DSH `govern` 与 NFR6 v2；不进入 `dist/` 产物集，不替代 `pactd`。
- **一致性保障**：`pact-engine-kernel` 共用 `Session`（D32）；wasm CI job 持续执行 wasm32 clippy/build、bindgen、`ci-smoke` 与 Node 7 项测试；语义改动须同时通过原生与 WASM 金标对账。
- **未变**：crate 拓扑、依赖方向、部署拓扑、事件/快照格式与 `dist/` 产物。

## 2026-10-21（第 21 轮，D32：新增 `pact-engine-kernel`，原生与 WASM 共用实例会话语义）

- **新增组件**：`pact-engine-kernel`（纯 crate，无 IO/async）承载**实例会话语义**——声明快照、Counter 状态、seq 分配、Rule-S/P/R/E 准备、Rule-T 终止判定、Rule-E 幂等表、恢复锚点与查询视图。它是语义层与编排层之间的共用内核，不持有存储、网络、时钟或通知能力。
- **依赖边**：`pact-engine-kernel → pact-core + pact-parser + pact-analysis + pact-store-model + pact-views`；`pact-engine → pact-engine-kernel`（另加 store/tokio）；`pact-wasm → pact-engine-kernel`（另加纯模型层 + wasm-bindgen）。依赖仍单向，无环。
- **组件角色调整**：原生 `pact-engine` 的 `InstanceSession` 与 `pact-wasm` 的 `InstanceSession` 均通过 `Deref` 嵌入同一个 `pact_engine_kernel::Session`；原生只保留 `events_since_snapshot`/`notify`，WASM 只保留 `events`。二者差异收敛到持久化时机、通知/审计与错误映射。
- **一致性**：事件 schema、JSONL 行、快照结构、seq 连续性与对外错误码均不变；原生与 WASM 的实例行为由同一份内核代码保证，消除约 300 行双实现漂移（R1）。
- **稳定化前置**：Q7 CI、金标扩面、`pact-engine-kernel` 抽取均已完成；仅余推送/发布前宿主边界安全审视。
- **未变**：crate 拓扑其余部分、单一状态变更通道（D6/D19）、引擎被动性（D5）、权威边界（原生 daemon 唯一权威）、部署拓扑与 `dist/` 产物。

## 2026-10-21（第 20 轮，P6 裁决 A：WASM 本地分发形态的组件边界登记（D31））

- **角色登记**：`pact-wasm` 由「spike S2」转为**实验性本地分发形态**——可嵌入 Node/桌面宿主的进程内单用户引擎；**权威仍是原生 daemon**，两者不并存于同一实例（宿主 lockfile 单写者）。
- **边界收紧**：WASM 形态不提供 MCP/HTTP 服务、不承担身份认证/ACL/读审计、不做 turso 索引与压缩、不进入 `dist/` 生产分发；多用户与远端能力仍归原生 daemon（FR17）与 NFR6 v2。
- **依赖与环境边界不变**：依赖方向仍为 `wasm → core + parser + analysis + compose + store-model + views`（`wasm-bindgen` 仅 wasm32）；`pact-engine`/`pact-store` 仅 dev-dependencies 用于原生对账，不进入产物。持久化与单写者由宿主适配层承担，磁盘布局与原生共用。
- **稳定化前置**：Q7 CI（wasm32 构建 + Node 冒烟 + 体积/ABI 断言）→ 金标扩面（多实例、组合域、条件分支、压缩恢复）→ 抽取 `pact-engine-kernel` 消除双实现 → 宿主边界安全审视；未完成前按实验性登记。
- **未变**：crate 划分、依赖图其他边、单一状态变更通道（D6/D19）、引擎被动性（D5）、线上格式、部署拓扑与 `dist/` 产物。本轮仅登记组件角色与边界，未改代码（配套：design-change 第 33 轮、requirements-change 第 18 轮）。

## 2026-10-21（第 19 轮，P4/S2：Node 宿主适配层）

- **新增层**：`crates/pact-wasm/js/` 作为 WASM 内核之外的**宿主适配层**（Node）：文件存储（JSONL/快照）、单写者 lockfile、日志优先的 `prepare → append+fsync → commit` 编排、最小 demo 与 `node:test`。它不改变 crate 依赖图，不进入 WASM 产物。
- **一致性**：磁盘布局、JSONL 行与快照结构与原生 `pact-store` 完全共用；已验证原生 daemon 与 WASM 宿主可交替打开同一日志（原生→WASM→原生→WASM）。
- **能力边界**：Node lockfile 为进程级近似单写者，不提供 `flock` 的内核级互斥；浏览器持久化（OPFS/SharedWorker）不在本轮。
- **未变**：crate 清单与依赖边、原生 API、需求与论文母本。验证：Rust 177 项 + Node 7 项全绿。

## 2026-10-21（第 18 轮，P3/S1：WASM 门面升级为内存引擎）

- **组件角色**：`pact-wasm` 从「无状态门面（解析/分析/组合）」升级为「内存引擎门面」——持有声明注册表、多实例会话、事件序列与幂等表，提供两阶段提交与日志/快照导出恢复；仍不承担磁盘、网络、线程与进程能力。
- **依赖边界**：生产依赖不变（`pact-core`/`pact-parser`/`pact-analysis`/`pact-compose`/`pact-store-model`/`pact-views`，`wasm-bindgen` 仅 wasm32）；新增 `pact-engine`/`pact-store`/`tokio`/`tempfile` 仅为 **dev-dependencies**，用于原生对账测试，不进入 WASM 产物。
- **一致性约束**：事件 schema、JSONL 行、快照结构与 seq 连续性仍以 `pact-store-model` + 原生 `Store` 校验为唯一权威；WASM 日志可被原生 daemon 直接打开，原生日志可被 WASM `restore` 读入（已验证）。
- **未变**：crate 清单、依赖图的其他边、原生 API、磁盘布局、需求与论文母本。验证：176 项测试全绿、fmt/clippy（native + wasm32）清零（详见 design-change 第 31 轮）。

## 2026-10-21（第 17 轮，P2/S0：WASM 门面组件（D30））

- **组件集新增**：`pact-wasm`（`cdylib + rlib`）。依赖方向 `wasm → core + parser + analysis + compose + store-model + views`，不依赖 `store`/`engine`/`http`/tokio，保持单向无回指。
- **边界**：spike S0 仅验证编译、wasm-bindgen 导出、体积与静态分析对账；不承担事件写入、持久化、`prepare/commit`，不进入生产分发与 CI（Q6/Q7）。
- **接口零变化**：原生 `pact-store`/`pact-engine`/HTTP/MCP 的类型与流程不变；新增组件只读取纯模型与视图。
- **验证**：171 项测试全绿、fmt/clippy（host + wasm）清零；wasm 产物 267 KiB / gzip 87 KiB，静态分析结果与原生 CLI 深度相等（详见 design-change 第 30 轮）。

## 2026-10-21（第 16 轮，P1：WASM 纯类型层拆分（D29））

- **组件集新增**：`pact-store-model`（事件/快照/实例键纯模型）与 `pact-views`（查询/分析视图纯类型）。依赖方向：`store-model → core`、`store → core + store-model`、`views → core + analysis`、`engine → core + store + views`，仍为单向、无回指。
- **接口与数据流零变化**：`pact-store`/`pact-engine` 通过 re-export 保持既有类型路径；磁盘布局、JSONL/线上格式、索引 schema、恢复与压缩流程、单一状态变更通道（D6/D19）、引擎被动性（D5）均不变。
- **新增可移植边界**：两个新 crate 不依赖 IO/async/tokio，可编译到 `wasm32-unknown-unknown`，为 WASM 本地 spike 提供共用模型层。
- **验证**：全量 166 项测试全绿、fmt/clippy 清零、两个新 crate wasm check 通过（详见 design-change 第 29 轮）。

## 2026-10-21（第 15 轮，阶段 6：CPN 编码生成离线组件）

- **§2.6 新增 CPN 编码生成数据流**：`.pact → pact-parser（解析 + A.5 校验）→ Domain → pact-cpn::generate（纯函数、零 IO）→ CPN Tools XML → pact cpn --out → model.cpn`。
- **组件集新增**：`pact-cpn` 仅依赖 `pact-core`；与 daemon、store、engine、http、mcp、runtime 完全解耦；CLI 离线命令新增 `cpn`，仍不触状态。
- **单实例作用域**：一次生成只编码一个 `Counter instance`，库所/迁移以 `i_<instance>__` 命名，跨实例无共享；符合论文 §4.5/§B.2 Scope 与 D27/G1。
- **无结构性变更**：crate 依赖方向仍单向（`cli → cpn → core`）、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层、进程/部署拓扑、线上格式均不变；本轮新增的是**离线只读生成器**，不进入任何运行时数据流。
- **验证**：生成物通过 CPN DTD 校验；全量 166 项测试全绿（详见 design-change 第 28 轮）。

## 2026-10-20（第 14 轮，阶段 9：分发产物落地）

- **§6 新增「分发形态（阶段 9 已完成）」**：`dist/` 提供三条部署路径——容器（多阶段 musl 构建、非 root uid 10001、`/var/lib/pact` 卷、`EXPOSE 8787`、`HEALTHCHECK`）、systemd（`pact` 用户、`StateDirectory`/`RuntimeDirectory`、20s 停机窗口、`ProtectSystem=strict` 等加固）、静态包（`x86_64-unknown-linux-musl` tar.gz + `SHA256SUMS`）。三者载荷为同一批静态二进制（`pact`/`pactd`/`pact-mcp`/`pact-runtime`）。
- **§6 部署图与归属边界更新**：图中 `dist/` 标注「阶段 9 已交付」；归属边界由「阶段 9 未开始」改为「已交付」，L1 生产化标「阶段 8 已交付」；§5 层边界行同步。
- **无结构性变更**：crate 划分、依赖方向（单向、无回指）、单机进程/部署拓扑、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变；本轮仅新增**部署/分发描述与产物**，不改数据流、启动恢复与任何代码路径。

## 2026-10-20（第 13 轮，D25 子项 3：域级 ACL 与读审计落地）

- **§2.5 新增读审计与域级授权数据流**：Rule-R 成功后由引擎 actor（单写者）向 `<domain>/<instance>.read-audit.jsonl` 追加一行 + fsync；失败即 `500`（fail-closed）；审计流不产生事件、不推进 `seq`、不参与重放 / Rule-T / 快照 / 压缩。REST 入口在 ACL 非空时由 Bearer 令牌反查参与者并按 `(domain, instance)` 授权（支持 `domain/*`），覆盖生命周期 / 动作 / 查询 / compact / 审计视图，`GET /domains` 按授权过滤。
- **组件内新增（不改变分层与依赖方向）**：`pact-store` 新增 `audit.rs`（`ReadAuditRecord`）与 `Store::append_read_audit` / `read_audit_tail`（独立文件 append + fsync；尾部读取）；`pact-engine` 新增 `EngineConfig.read_audit`、Rule-R 审计写入、`Command::QueryReadAudit` + `Engine::read_audit`；`pact-http` 新增配置 `read_audit` / `[auth.acl]` + `Config::validate`、`ServerState::with_acl` 与鉴权辅助、读审计视图路由、client 方法。`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。
- **§5 质量属性更新**：安全边界行改为「启用 ACL 后全端点实例授权（401/403、列表过滤、配置校验），未启用保持信任声明」；可运维性行由「子项 1+2 已实现」扩为「**子项 1+2+3 已实现**」，并登记可选读审计流与只读视图。
- **无结构性变更**：crate 划分、依赖方向、单机单进程部署、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变。本轮变化限于**接口层鉴权 / 审计旁路 + 存储新增独立文件族**。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **155 项全绿**（基线 145 + 本轮 10）。

## 2026-10-20（第 12 轮，D25 子项 1：日志压缩与 schema 版本化落地）

- **组件内新增（不改变分层与依赖方向）**：`pact-store` 新增压缩能力——`WireEvent{version, #[serde(flatten)] event}`（事件与线上格式统一带顶层 `version`）、`logmeta` 水位旁路清单（`<instance>.logmeta.json`）、`rewrite_retaining`（tmp+fsync+rename 原子前缀重写）、`IndexStore::prune_events`、`Store::compact`；`Snapshot` 扩为**自足锚**（`version`/`seq`/`dl_seq`/`declaration`/`state`/`consumed_requests`）。`pact-engine` 增 `Command::Compact` + 公开 `Engine::compact` + `EngineConfig::compact_on_start/compact_keep_last` + `recover_instance` 的 `snapshot_is_anchor` 分支；`Notification` 增顶层 `version`。`pact-http` / `pact-cli` / `pact-mcp` 仅**增入口**（`POST .../compact`、`pact compact`、`pact_compact`）。`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。
- **§1 架构总览图更新**：存储层框补 `.logmeta.json（压缩水位）`，`.snapshot.json` 标注为「自足锚」。
- **§2.3 启动恢复流程更新**：补入 logmeta 水位读取、快照锚/重放双分支、`SnapshotRequiredForCompactedLog` 护栏、`base_seq > 0` 时索引补 `prune_events` 自愈、`compact_on_start` 自动压缩、事件 schema 版本兼容规则。
- **新增 §2.4 日志压缩数据流**：五步序（写自足快照为提交点 → 写 logmeta 水位 → 原子重写日志 → 裁剪索引 → 重置追加句柄），水位单调不回退；崩溃于任一中间点落到两个一致态之一（未截断超集态 / 已截断+水位+快照）。
- **§5 质量属性更新**：审计完整性行补「压缩后审计视图 = 自足快照 + 保留尾部事件，被丢弃前缀不再可查」；可恢复性行补「快照锚恢复 + 水位护栏 + 版本 fail-fast」；可运维性行由「子项 2 已实现」扩为「**子项 1+2 已实现**」。
- **无结构性变更**：crate 划分、依赖方向（单向、无回指）、单机单进程部署拓扑、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变。本轮变化限于**存储层文件族 + 恢复流程分支 + 接口面新增一个运维端点**，属既有分层内的落地细化。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **145 项全绿**（基线 131 + 本轮 14）。

## 2026-10-20（第 11 轮，D25 子项 2：运维接口落地）

- **§5 可运维性行**（由「未实现」转**子项 2 已实现**）：daemon 增加三个**免令牌**运维端点 `/health`、`/ready`、`/metrics`（Prometheus 文本），可选 `pid_file`（陈旧检测），SIGINT/SIGTERM 优雅停机（停止 accept → 有界 10s drain → 终态快照）。明确**未实现**：日志压缩/保留与 schema 版本、域级 ACL、读审计扩展。
- **组件内新增**（不改变分层与依赖方向）：`pact-http::server` 增加 `ConnTracker`/`ConnGuard`（在途连接计数，供有界 drain）与 `watch` 停机广播；`serve_unix` / `serve_tcp` 签名增加停机接收器与计数器，**仅 `pact-http` 内部与测试调用点**受影响。`pact-engine` / `pact-store` / `pact-core` 零改动，依赖方向不变。
- **无结构性变更**：crate 划分、部署拓扑（单机单进程）、引擎被动性（D5）、单一状态变更通道（D6/D19）均不变。

## 2026-10-20（第 10 轮，术语精化：「域实例」）

- **触发**：用户要求把指 D27 实例键的「实例」统一写「域实例」（避免与工程实例混淆，为将来类型-实例分离留余地）；见 `docs/design-change.md` 第 21 轮。
- **改动**：`architecture.md` §2.3 启动恢复步骤「逐实例（<domain>/<instance>.events.jsonl）」→「**逐域实例**（…）」。§5 多实例承载行的「Counter 实例」「实例键」等为论文/机制术语，保留不改。
- **形式层与实现零变化**：仅中文措辞；`docs/pact*` 母本未触碰。

## 2026-10-20（第 9 轮，阶段 10 实施：域双键实例化落地）

- **§5 多实例承载行更新**（由第 8 轮的「未实现」转**已实现**）：补入**存储布局定案**——域子目录 `<data_dir>/<domain>/<instance>.events.jsonl`（快照 `‹instance›.snapshot.json`、声明原文 `declaration.pact` 同目录）；索引 `(domain, instance)` 双列（`events`/`tokens` 复合主键）；顶层遗留单层文件 fail-fast（`LegacyLayoutDetected`）；同一 `data_dir` 以 flock（`‹data_dir›/pactd.lock`）保证单写者；非空实例再加载默认拒绝（`DomainBusy`，须 `force`）；实例键以路径段承载（`/domains/{d}/instances/{i}/…`，`/domains/{d}` ≡ `default`），键名谓词 `[A-Za-z0-9_-]{1,128}` 防路径穿越。
- **§1 架构总览图更新**：引擎层框由「域会话表（`domain → {Domain, CounterState, 订阅者}`）」改为「**声明注册表（`domain → 声明`）+ 实例会话表（`(domain,instance) → {声明, 状态, 订阅者}`）**」；存储层框由「`<domain>.events.jsonl` / `<domain>.snapshot.json` / `index.tursodb`」改为「`<domain>/<instance>.events.jsonl` / `<domain>/<instance>.snapshot.json` / `<domain>/declaration.pact` / `index.tursodb (domain,instance) 双列`」。
- **§2.3 启动恢复流程更新**：补入「先取 data_dir 独占 flock（`pactd.lock`，第二实例拒绝启动）」「顶层遗留单层文件 → fail-fast（`LegacyLayoutDetected`）须人工迁移」「逐**实例**恢复」「重放后恢复**声明注册表**（读 `<domain>/declaration.pact`）」。
- **无结构性变更**：crate 划分、依赖方向（单向、无回指）、单机进程/部署拓扑、单一状态变更通道（D6/D19）、引擎被动性（D5）、纯函数语义层均不变；`pact-core` / `pact-parser` / `pact-analysis` / `pact-compose` 零改动。本轮变化限于**存储布局细节、引擎状态表形态与接口路由形状**，属既有分层内的落地细化。
- **质量属性关联**：审计完整性行维持（`DomainLoaded` 存档声明原文）；可恢复性行维持（索引用 `(domain,instance)` 双列，可重建自愈）。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **123 项全绿**（基线 110 + β 专项）。

## 2026-10-20（第 8 轮，D27 域双键实例化立项：形式层裁定与论文 rev3）

- **本轮无结构性变更**：crate 划分、单机进程/部署拓扑、单一状态变更通道（D6/D19）、数据流、纯函数语义层均不变；引擎被动性（D5）不变。**零代码改动**。
- **§5 质量属性表新增一行**「多实例承载（FR19，D27，未实现）」：同一域声明可并发承载多个 Counter 实例；实例为接口层一等键，各实例独立持有 Counter / Archive / 事件日志 / 快照 / 审计流 / 订阅通道 / Rule-E 幂等表；同一 `data_dir` 上多实例须以 flock 保证单写者；存储布局（单层文件名 vs 域子目录）待裁决。
- **背景**：域名今天既是定义标识又是会话键，`LoadDomain` 的替换语义（design.md §8.1）在两个并行项目共用同一域声明时会**销毁**对方 Counter 与快照——这是 D27 立项的直接动因。评估见根目录 `model-c-impact.md`（五层 × 三档判定）。
- **形式层依据**：领域专家裁定该拆分**不构成形式冲突**，属论文 §1.3「Physical implementation of the Counter」既有的实现自由（Q1 (a)）；论文据此修订为 **rev3**（9 处增补/术语替换，零文法、零语义变化，`diff -u` 已核），母本引用全仓同步至 rev3。裁定要点见 `docs/design-change.md` 第 18 轮。
- **架构影响待评估**：存储布局（单文件 vs 子目录）、引擎会话表形态（声明注册表 + 实例会话）、事件日志与索引的实例维度、REST 路由形态——均待 3 项细节裁决后在「阶段 10 域双键实例化」实施时评估并回填本文件 §4（数据流 / 启动恢复）与 §5。
- **可运维性行（NFR7/D25）**仍标"未实现"（属阶段 8），且其验收与实例化耦合（见 `model-c-impact.md` §6）。

## 2026-10-19（第 7 轮，阶段 7：L3 生产运行层落地）

- **§6 落地核对**：标题去"未实现"、增注"L3 运行层已实现"；新增「落地形态（阶段 7 已完成）」条——`crates/pact-runtime/` 交付传输抽象（`ParticipantClient` / `RestClient` / `McpClient`）、驱动循环 `drive()`、编排 `Orchestrator`、副作用执行器 `SideEffectExecutor`、TOML 配置驱动 bin；部署图标注由"（目标）"改为已交付（L3 lib + bin）。
- **§6 依赖方向细化**：由 `pact-runtime → pact-mcp / pact-http` 修正为 `pact-runtime → pact-http / pact-engine / pact-core`（lib 层）；`pact-mcp` 仅作可执行文件由编排 spawn、不作 lib 依赖——与 design.md §1 依赖行一致（D24 行依赖细化）。
- **§5 质量属性更新**：层边界行"生产驱动逻辑收于新增 `pact-runtime`…"改为"已实现，经 REST 接入 daemon"。
- **无结构性变更**：crate 划分（除 `pact-runtime` 由"未实现"转"已实现"，见 design.md §1）、单机进程/部署拓扑、单一状态变更通道、数据流、纯函数语义层均不变；引擎被动性（D5）不变。可运维性行（NFR7/D25）仍标"未实现"（属阶段 8）。
- **验证**：`cargo fmt --all --check` 干净、`cargo clippy --workspace --all-targets -- -D warnings` 零警告、`cargo test --workspace` **110 项全绿**（基线 105 + 本轮 5）；engine/store/core/三接口零改动。

## 2026-10-19（第 6 轮，生产运行层与生产化边界）

- **新增 §6「生产运行层与分发」**：在既有单机架构之上补 **L3 生产运行层**（新增 crate `pact-runtime`），给出目标进程拓扑图、依赖方向（`runtime → mcp / http`，不反向）、进程拓扑（systemd 托管的常驻编排进程拉起 daemon 与 agent 子进程；MCP 仍由 agent 宿主以 stdio 子进程拉起）、归属边界与 MCP 提供形态（权威恒为 daemon）。
- **§5 质量属性增两行**：可运维性（NFR7 / D25，未实现）、层边界（D24–D26）。
- **无既有结构变更**：crate 划分（除新增 `pact-runtime`，见 design.md §1）、依赖方向、单机进程/部署视图、数据流、单一状态变更通道均不变；§2 数据流、§3 v1 部署视图、§4 联网演进不动。
- **未实现标注**：§6 与 §5 新增行均标注"未实现"，本轮仅为架构裁决落档（对应 design.md D24–D26、requirements.md FR16–FR18）。

## 2026-10-18（第 5 轮，提交契约与事件自足落地核对）

- **§2.1 数据流补正**：Rule-P 路径标注"追加 JSONL(fsync，**提交点**) → 派生索引投影(turso，尽力而为) → state.apply"，补提交点前后失败语义（提交点前无部分效果、提交点后不回滚）与 Rule-E 可选幂等键说明（对应 design.md D23）。
- **§5 质量属性两行更新**：审计完整性行补"每条事件自足（Consumed 携 token 全量快照），单行即可审计与重放"；可恢复性行补"索引滞后由启动一致性校验触发重建自愈"。
- **无结构性变更**：crate 划分、依赖方向、进程/部署拓扑、单一状态变更通道、纯函数语义层均不变；本轮仅数据流时序标注与质量属性表述随实现契约补正。

## 2026-09-04（第 4 轮，阶段 4：接口层落地）

- **接口接入拓扑变更（D19）**：pact-mcp 与 pact-cli（在线）不再规划内嵌引擎，统一作为 pact-http daemon 的 REST 客户端；REST 通道成为通往引擎的唯一状态变更路径，三接口能力等价由构造保证。§1 总览图按此重绘（含图例更新：MCP stdio 为双向 JSON-RPC 流，箭头仅示发起方）。
- **组件集变更**：原 `pact-server` 组装 bin 取消；可执行物收敛为三个静态二进制——`pact`（CLI + daemon start）、`pactd`（裸 daemon）、`pact-mcp`（stdio MCP server）。§1/§3 相应更新。
- **传输范围收窄（D20）**：MCP v1 仅 stdio；Streamable HTTP 随联网演进（v2）落地，§3 部署视图移除该行。
- §2.1 数据流补上 REST 一跳（pact-mcp → REST → pact-http → engine）；§2.3 启动恢复主体由 `pact-server` 改为 `pact daemon start / pactd`。
- 结构类别覆盖不变：模块视图（§1 分层 + crate 依赖，见 design.md §1）、C&C 视图（§1/§2）、分配视图（§3）各一；无新增质量属性手段（§5 仅既有属性落点不变）。

## 2026-08-26（第 3 轮，阶段 3 落地核对）

- 清除 §2.1/§2.3/§3 残留的三处 `index.sqlite`/SQLite 旧称，统一为 `index.tursodb`/派生索引（turso）（D13 早已决策，本轮落地实现后核对补正）。
- §2.3 启动恢复流程按实现补全：快照有效性条件（seq ≥ 最后一次 DomainLoaded）、中部损坏行 fail-fast 语义（对应引擎恢复失败统一应答）。
- 阶段 3（pact-store + pact-engine）实现完成，架构图与数据流描述与代码一致，无结构性变更。

## 2026-08-25（第 2 轮）

- 存储层派生索引引擎由 rusqlite/SQLite 改为 turso（设计决策 D13），存储布局 `index.sqlite` → `index.tursodb`，经 `IndexStore` trait 隔离。
- 架构总览图增加图例（箭头语义、分层说明），对齐 documenting-software-architectures 技能的七规则之三（避免歧义）。
- 质量属性表更新：语义收口至 core（D14）、四层测试组织、索引可重建性表述。

## 2026-08-25（第 1 轮）

- 初始架构建档（分层视图、数据流、部署视图、联网演进、质量属性落点）。
