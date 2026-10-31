# 物引治理：从静态制品到动态能力

对应规范：`paper/pactv1_rev3.md`（下称“母本”）§1.2、§1.3、§1.4、§3.2、§4.4，以及 `docs/expressiveness-boundaries.md` 的 B11、B12、B20、X4。

状态：专题方案（2026-10-26）。本文件讨论“ThingRef 应如何治理”，不改变母本；涉及母本文法或规则的修改，仍须用户显式确认。

结论先行：

- **ThingRef 治理的对象不是字符串，而是交接承诺。** 一张物引 = 票面（URI/Identifier）、契约（ContractCard）、通道（resolver/invoker）、信任（auth）、时效（retention）。
- **范式内只保留不透明边界，治理放在范式外。** 引擎只登记票根，不负责提货；Counter、Rule-P、GateExpr 永远不解析 ThingRef。
- **票面不强制 URI。** 保留母本 §3.2 的 `URI | Identifier | ∅`；新引用优先 URI，旧 Identifier 兼容并标记为“未分类引用”。
- **匹配在本地，解析在远端。** `(domain, type, scheme) → RefBindings` 是纯函数；真正 fetch / invoke 才需要治理平面。
- **PACT 管“谁在什么条件下可以做什么”，治理平面管“交给你的到底是什么、怎么取、能不能信、过期怎么办”。** 两者只在 token 的 `TypeName` / `ThingRef` 边界相遇，不互相吞并。

阅读顺序：§1 根问题 → §2 顶层模型 → §3 票面与 `∅` → §4 静态形态 → §5 动态形态 → §6 契约梯度 → §7 与域声明的耦合 → §8 与参考实现的耦合 → §9 新项目与 GitHub 管理 → §10 决策记录 → §11 契约卡示例 → §12 反模式与驳回 → §13 落地顺序与验收 → §14 引用位置。

---

## 1. 根问题：一个字段承担了五种角色

| 角色 | 回答什么 | 今天在哪里 |
|---|---|---|
| 指称 | 指的是哪一个 | `ThingRef` 字符串 |
| 能力 | 怎么取、怎么调 | 隐含在 scheme 或团队约定里 |
| 契约 | 下游能依赖什么 | 基本缺失，靠默契 |
| 凭据 | 凭什么允许取 | 常被塞进 URL、环境变量或调用方代码 |
| 时效 | 过期、删除、换版怎么办 | 基本缺失 |

一个字符串同时承担这些，必然出三种事故：

1. 下游拿到 ref，却不知道怎么用；
2. 能用，却不知道拿到的是不是承诺的那一版；
3. 上游 token 进了 Archive，制品却已删除、换版或无权限。

所以治理的第一步不是定语法，而是承认这五件事可以分离。**ThingRef 治理的对象不是字符串，而是交接承诺。**

---

## 2. 顶层模型：提货单五层

### 2.1 五层

| 层 | 治理对象 | 归属 | 引擎可见性 |
|---|---|---|---|
| 票面 | scheme / locator / 可选 revision 或 digest | 生产者 | 只透传 |
| 契约 | kind、media type、schema、version、guarantees | 生产者 / 领域作者 | 不可见 |
| 通道 | resolver / invoker / adapter | 制品域 / 平台 | 不可见 |
| 信任 | mint 权、resolve 权、audience、revocation | 安全域 | 不可见 |
| 时效 | retention、TTL、tombstone、Archive 对齐 | 制品域 + 域约定 | 不可见 |

三句话原则：

- **类型管可判定，物引管可寻址。**
- **契约管可依赖，解析器管可兑现。**
- **引擎管“有没有这枚 token”，不管“它指的东西在不在”。**

### 2.2 三方

- **生产者**：承诺“我指的是这个、这一版、下游能取到”。
- **保管者**：承诺“在保留期内可解析、权限可控、删除可审计”。
- **消费者**：承诺“解析失败、版本漂移、无权限时显式失败，不假装成功”。

引擎不是第四方，它只是**登记员**：记录“这枚 token 存在过、票面是什么”，不参与兑现。

### 2.3 六条硬约束

以下约束是治理层必须守住的边界。它们不是新的范式规则，而是“不要越过母本”的工程约束。

1. **不透明约束**：Counter、Rule-P、GateExpr 永不解析 ThingRef。需要 referent 敏感的判定，编进 `TypeName`，或用 witness token（B11/B12）。不要在 gate 里读内容，那是 X4。
2. **无秘密约束**：ThingRef 会进入事件日志、Archive、CPN 颜色集、读审计和可能的 SSE。不得含 bearer token、密钥、个人敏感信息。动态能力用间接引用 + 凭据代理。
3. **定版约束**：进入 Archive 或跨边界交付的 ref，应指向不可变 revision 或携带内容摘要。可变名称可以作为工作态引用，但不能作为唯一交付凭据。
4. **可发现性约束**：下游在行动时必须能获得票面；上游被消费后，Archive 的保留期和访问权必须覆盖下游。否则就是“盲生产”。
5. **无副作用约束**：引擎不 fetch、不 invoke、不校验存在性、不重试。动态调用只能由参与者执行。
6. **显式失败约束**：解析失败、无权限、已过期、版本漂移，都必须由消费者显式处理；引擎不猜测、不降级、不自动补偿。

---

## 3. 票面与 `∅`

### 3.1 形态决策：URI-first，Identifier-legacy

母本 §3.2 允许 `ThingRef ::= URI | Identifier | ∅`。本专题**不强制 URI**，但建议：

- 新引用一律优先使用 URI：`scheme + authority + path + 可选 revision/digest`。
- 历史或内部短 ID（如 `REQ-001`）可以作为 `Identifier` 保留，但标记为“未分类引用”，lint 提示补 scheme。
- 裸路径（`/tmp/a.md`、`C:\...`）不作为跨参与者物引；本机路径对下游没有意义。至少用 `file://` 并声明共享前提。
- `∅` 表示“确认没有外部制品”，不是“解析失败”。

**撤回记录**：曾考虑“强制 URI、不考虑向后兼容”，2026-10-26 撤回。原因：母本允许 Identifier；强制 URI 需要改母本，收益不足以抵消兼容与迁移成本；URI-first 已能获得主要治理收益。

### 3.2 scheme 注册表

Scheme 注册表可以只是文档表，不一定是服务。新增 scheme 必须登记语法、解析器、安全说明和一致性测试。

| 类别 | 例子 | 兑现方式 | 必须声明 |
|---|---|---|---|
| 不可变内容 | `oci://...@sha256:...`、`git://...@commit`、`https://...#sha256:...` | 取回 | media type、digest |
| 可变位置 | `https://...`、`s3://...` | 取回 | 版本/ETag、保留期、是否允许漂移 |
| 命名空间 | `urn:...`、`task://...` | 查注册表 | 命名权威、解析器 |
| 本机/共享文件 | `file://...` | 直接读 | 共享前提、生命周期 |
| 动态能力 | `mcp://...`、`tool://...` | 调用 | 能力契约、auth、副作用 |
| 缺席 | `∅` | 无 | 与“未提供”的区分策略 |
| 未分类 | `REQ-001` | 依上下文 | 只能作为 Identifier；lint 提示补 scheme |

### 3.3 `∅` 与“未提供”的区分

**决策：区分发生在入口，不发生在 token。**

- 请求字段缺失 → 400，视为作者漏传。
- `thing_ref: null` → `∅`，视为“确认无制品”。
- `thing_ref: "..."` → 有引用。
- token 只有两态：无引用 / 有引用。不引入缺失、null、URI 的 tri-state。

配合 `RefBindings` 的 `required`：

- `required: true` 的类型，`null` 也拒绝；
- `required: false` 的类型，`null` 合法；
- 不在 token 结构里增加第三态。

这样既能抓漏，又不会把事件 schema、CPN、serde 和所有消费者复杂化。

---

## 4. 静态形态的治理

### 4.1 基本原则

- **不要把 URL 当契约。** URL 只回答“去哪找”，不回答“找到的是什么、能不能信、过期怎么办、谁有权取”。
- **可变引用要显式标记。** 归档时冻结为 digest/version。
- **复合制品用 manifest ref 或拆分 token。** 不要把多个 locator 拼进一个字符串。
- **禁止 userinfo。** `https://user:pass@...` 直接拒绝；凭据只从 broker 取。
- **长度上限。** 建议 2048 或 4096，防止日志和事件膨胀。
- **scheme 白名单。** 默认只允许注册过的 scheme；`file:`、自定义 scheme 要显式开启，并做 SSRF/路径穿越防护。

### 4.2 静态类别

| 形态 | 适合 | 风险 |
|---|---|---|
| 不可变内容（digest/commit） | 归档、跨组织交付、审计 | 需要内容寻址基础设施 |
| 可变位置（https/s3） | 工作态、短周期协作 | 下游可能拿到不同版本 |
| 命名空间（urn） | 内部 ID、任务/记录引用 | 需要注册表解析 |
| 共享文件（file://） | 同机/同挂载 | 下游可能不可见；路径穿越 |
| 复合（manifest） | 多文件交付、整体版本 | 部分访问需额外协议 |

### 4.3 复合制品

**决策：manifest 承载“包”，独立 token 承载“需要独立准入/消费的部分”。**

- 如果所有部分共享同一 gate 和生命周期 → 一个 manifest ref。
- 如果某部分要被单独读/消费/审计 → 单独 token。
- 不要把多个 locator 拼进一个 ThingRef。
- 不要指望一次 Rule-P 原子产生多枚 token（B15/X3）。

收益：整体交接清晰，局部治理不被牺牲。代价：manifest 本身也是制品；需要 manifest schema 和解析器；包与部分可能不一致，需要 lint/校验。

---

## 5. 动态形态的治理

### 5.1 动态物引是能力引用，不是另一种地址

先分三类，因为治理强度不同：

| 类型 | 例子 | 语义 | 治理要点 |
|---|---|---|---|
| 取回型 | 查询 API、MCP read tool | 调用即读取，理想情况幂等 | 可当惰性静态引用；仍需版本/schema 钉住 |
| 生成型 | build、deploy、submit | 调用有副作用 | 不得由 consume 隐式触发；用 witness/outbox/saga |
| 长时型 | job、stream、异步任务 | 返回 handle 而非结果 | 先物化 job handle；状态查询另走协议 |

动态票面至少应能回答：

- 谁是服务方（server identity，不只是 URL）
- 调用哪个操作（tool/operation）
- 契约版本与 schema 摘要
- 副作用类别（pure / read-only / mutating）
- 幂等键与重试语义
- 认证受众与权限范围
- 超时、失败码、结果物化方式

**动态调用是“提货方式”，不是“货”本身；没有物化退路的动态物引，不具备归档资格。**

### 5.2 MCP 的两档 profile

不建议只定一个“最小 profile”。建议两档：

| 档位 | 允许的字段集 | 适用 |
|---|---|---|
| `mcp-read` | server identity + tool + schema digest + auth audience + read-only + idempotent + result materialized | 只读、幂等、结果可物化的查询工具 |
| `mcp-mutating` | 上一档 + side-effect class + idempotency key + timeout + error taxonomy + audit + materialization policy + retention | build/deploy/submit 等有副作用的工具 |

MCP 的 `readOnlyHint` / `destructiveHint` / `idempotentHint` 只能当**提示**，不能当契约保证。契约卡必须显式声明，不能只信工具自描述。

收益：低风险工具低门槛，高风险工具高保证。代价：两套 profile 要维护；工具作者要分类；分类错了比没有更危险。

### 5.3 动态调用的证据

- 治理平面或参与者 SDK 可以记录可选审计边车：`trace_id / token_id / participant / ref / contract / operation / side_effect / request_id / status / result_digest / started_at / ended_at`。
- 不记录正文、凭据、个人数据。
- 默认关闭，开启时按 retention 治理。
- 审计边车放在治理平面或 `pact-runtime`，**绝不放进 engine/core**。
- 结果需要长期依赖时，必须物化成静态 ref，再用正常 `seed/produce` 产生新 token。动态 ref 不应成为 Archive 里的终态。

---

## 6. 契约梯度：允许默契，但要设升级触发器

### 6.1 三档

| 档位 | 形式 | 适用 | 失败代价 |
|---|---|---|---|
| 默契级 | 命名约定 + 团队知识 | 同团队、短周期、低爆炸半径 | 自己承担 |
| 声明级 | 一页 ContractCard：kind、media type、version、owner、retention、auth、failure | 跨角色/跨仓库、有自动化消费 | 可审计 |
| 可验证级 | schema + digest + 签名 + 一致性测试 | 跨组织、长期归档、合规/高风险 | 可机器验收 |

### 6.2 升级触发器

满足任一条件，就应从默契升级：

- 跨组织/跨信任域。
- 进入 Archive 且下游可能晚于上游消费。
- 由自动化参与者消费。
- 涉及部署、资金、隐私、合规。
- 需要版本演进而不破坏下游。

### 6.3 契约与类型的边界

**契约里影响准入的有限分类，必须进 `TypeName`；开放的内容描述留在 ContractCard。**

否则就是绕过 B11/B12 做内容寻址。TypeName 是 PACT 的语义面；ContractCard 是交接面。两者不能互换。

---

## 7. 与域声明的耦合

### 7.1 今天的事实：语法层零耦合

当前 `.pact` 的四个注入点只有 `types / preconditions / gates / seed_authority`，没有 ThingRef 值。parser 和 `pact-core::Domain` 完全不认识 `ThingRef`；它只出现在 token 结构和 Rule-S/Rule-P 的入参里。

| 关系 | 耦合度 | 说明 |
|---|---|---|
| `.pact` 文法 ↔ ThingRef | 零 | 域声明不写物引值，也不声明物引格式 |
| 域类型 ↔ 物引 | 弱 | 只有运行时 token 把它们放在一起 |
| 域语义 ↔ 物引内容 | 零 | Rule-P/GateExpr 不读 ThingRef |
| 域部署 ↔ 物引治理 | 可选弱耦合 | 通过旁车清单建立 `(domain, type) → profile` 映射 |

结论：**ThingRef 治理不需要改 `.pact` 语法。**

### 7.2 匹配方式：RefBindings 旁车

建议在域声明旁边放一份 **RefBindings** 清单，按 `(domain, declarationDigest, type, scheme)` 匹配。它是可选的；没有它时按 scheme 默认策略走。

匹配优先级：

1. `(domain, instance, type, scheme)` 精确匹配。
2. `(domain, type, scheme)`。
3. `(domain, type, *)`。
4. `(scheme)` 全局默认。
5. 全局默认；若 `required=true` 且无匹配 → 拒绝。

匹配链：

```text
Token { domain, type, thing_ref }
   │
   ├─ (domain, type) ──► RefBindings ──► 允许的 scheme / contract / required / visibility
   │
   └─ thing_ref ──► parse URI 或 Identifier ──► scheme adapter ──► ContractCard ──► resolver / invoker
```

### 7.3 三层重用，一层不重用

| 层 | 是否可重用 | 说明 |
|---|---|---|
| Scheme adapter（https / git / oci / mcp / file…） | 高度可重用 | 全平台共用，新增 scheme 只加一个适配器 |
| ContractCard（某类制品的契约） | 可跨域重用 | 例如 `se/design-review@2` 可同时被多个域引用 |
| Validator / lint / 解析器 | 可重用 | 纯函数，放哪儿都能用 |
| RefBindings（域内类型 → profile） | 域专属，但很薄 | 可按组织默认 → 域默认 → 类型覆盖继承 |
| 域语义（什么类型表示什么事实） | 不可重用 | 这是 PACT 类型系统的事，不能塞进治理层 |

一句话：**治理层可重用，域语义不可重用；不要把两者的边界模糊掉。**

### 7.4 不要做的事

- 不要把 contract / scheme 写进 `.pact` 语法。契约是开放、会演进的；`.pact` 是有限、静态的。
- 不要让 gate 读 ThingRef 做契约匹配。需要影响准入的有限分类，仍然进 `TypeName` 或 witness token。
- 不要把 `(domain, type) → contract` 做成唯一全局映射。同一类型可能允许多种 scheme/contract；映射应是“允许集合”，不是单值。

---

## 8. 与当前参考实现的耦合

### 8.1 逐 crate 影响

采用“核心保持不透明，入口做治理校验”的路线：

| crate / 层 | 改动 | 说明 |
|---|---|---|
| `pact-core` | 几乎不改 | `ThingRef(Option<String>)` 保持不变；URI 校验不放内核 |
| `pact-parser` | 零改动 | `.pact` 文法不变 |
| `pact-analysis` | 新增摘要工具 | `digest` 纯函数计算声明语义接口摘要（FR34/D49），CLI 与未来运行期共用；不进入核心分析流程 |
| `pact-cpn` | 零改动 | ThingRef 仍是颜色集里的字符串 |
| `pact-store-model` | 可不变 | 字段形状不变；若要彻底不兼容才 bump 版本 |
| `pact-engine-kernel` | 零改动 | 只透传字符串 |
| `pact-engine` | 零改动 | 不解析、不 fetch、不校验存在性 |
| `pact-http` | 入口校验 + 视图可见性 + 视图审计 + 摘要比对 | `/seeds`、`/tokens` 校验；`/counter`、`/archive` 已实现默认 `read-gate` 过滤，RefBindings 的 `domain`/`hidden` 覆盖（FR28/D42）；视图级读审计已实现（`read_audit` 记录 `kind=view`，D43）；`POST /domains` 的 `declarationDigest` 运行期比对已实现（D49 第二步，`[ref_digest] mode = off/warn/enforce`） |
| `pact-mcp` | 入口校验 | 与 REST 同一套校验；不新增治理语义 |
| `pact-cli` | 校验 + 离线 lint + 摘要 | `--thing-ref` 校验；`pact ref lint` 已实现（FR27/D41），离线读 RefBindings/ContractCard/`.pact`；`pact ref digest` 与 lint 摘要比对已实现（FR34/D49 第一步） |
| `pact-wasm` | 入口校验（写入边界） | 复用 `pact-ref-spec::validate_entry` 纯函数，在 `prepare_seed` / `prepare_produce` 进入内核前做三态、URI profile 与绑定策略校验（D50）；宿主注入 RefBindings，解析器/调用器不进 |
| `pact-runtime` | 参与者侧治理 | resolve / invoke / 审计边车最适合放这里 |
| `web/authoring` | 提示与校验 | 不涉及引擎 |

结论：**真正的耦合点在入口层和参与者运行时，不在引擎。**

现状（2026-10-31）：`pact-http` 已实现入口与归档校验——`POST /seeds`、`POST /tokens` 在引擎调用前完成三态判定、URI profile 与 RefBindings 策略（FR30/D45）；`POST /consume` 在进入 Archive 前强制 `pinning: required-at-archive`（FR31/D46）与绑定引用卡片的归档钉版要求（FR32/D47）；`value` 与 `onUnknownScheme` 也在写入时生效。
`/counter`、`/archive` 的视图过滤与视图审计已实现（FR28/D42、D43）。`pact-cli` 的离线 `ref lint` 已实现（FR27/D41）；`declarationDigest` 的计算工具与 lint 漂移比对已实现（FR34/D49 第一步，规范见 `spec/ref/v1/declaration-digest.md`）。
运行期比对已实现于 `POST /domains`（FR34/D49 第二步，`[ref_digest] mode = off/warn/enforce`；`enforce` 不一致返回 `400 DeclarationDigestMismatch`）。
`pact-wasm` 本地宿主也已接入写入侧入口校验（FR30/D50）：`prepare_seed` / `prepare_produce` 在进入内核前调用同一 `validate_entry` 纯函数；宿主通过 `refBindings` / `PACT_WASM_REF_BINDINGS` 注入 RefBindings，未注入时只做结构规则。WASM 省略与 `null` 都表示 `∅`，不产生 `ThingRefMissing`；ContractCard 与 consume 前的归档钉版检查仍只在 REST 边界。
待做项与优先级见 `todo.md`（与 §13.1 对齐）。

### 8.2 单 WASM 是否仍能支撑

| 能力 | 单 WASM 能否支撑 | 原因 |
|---|---|---|
| 协调内核（token、规则、事件、快照） | 能 | 现有 `pact-wasm` 已经做到；无 IO |
| 静态治理（URI 校验、RefBindings 匹配） | 已实现 | 纯函数；宿主以 JSON 注入绑定（D50），未注入时只做结构校验 |
| ContractCard 校验与归档钉版 | 部分 | 绑定声明的卡片规则可进 WASM，但卡片加载与 consume 前归档钉版仍只在 REST 边界 |
| 解析/调用（fetch、MCP invoke、OAuth、撤销、审计） | 不能 | WASM 当前明确不做网络/安全边界；浏览器也不能当 ACL/审计边界 |
| 注册表/发现/多租户 ACL | 不能 | 这是服务面，不是嵌入式内核面 |

所以：

> **单 WASM 仍能支撑“协调内核 + 静态治理”，但不能支撑完整的治理平面。**
> 如果坚持单 WASM 自足，就只能接受“只能解析预置的静态制品、不能动态调用、不能做撤销与审计”的本地模式。

不建议把网络调用塞进 WASM。那会破坏当前 `pact-wasm` 的“无 IO、宿主负责持久化”的边界，还会把异步、凭据、CORS、撤销带进一个本来只做语义的内核。

### 8.3 当前实现里的一个真实缺口

当前 `GET /counter` / `GET /archive` 在域级授权后就能看到 token 和 `thing_ref`，不经过 participant gate。`CounterView` 直接包含 `Vec<Token>`，`ArchiveView` 直接包含 `ArchiveEntry`；HTTP 处理器只调用 `authorize_key`。

这意味着“读 gate”和“观察视图”是两套可见性。治理必须明确以哪套为准，否则会出现“gate 拦住了 read，但视图泄露了票面”。

**决策：默认 `read-gate`，显式 `domain` 才公开，`hidden` 给敏感引用。**

- 视图返回 token 存在性，但 `thing_ref` 只在调用者对该类型有 `reads` 或 `consumes` 静态 gate 时返回。
- 这是静态判断，不需要逐 token 求值；但若该子句带 `via` / `when` 条件，静态存在性会高估权限。此时应取保守策略：条件子句不满足可见性，或逐 token 求值后再返回。
- 实现上需要一个按调用者过滤的 token DTO，或视图层构造 `thing_ref = ∅` 的副本；不能直接序列化全量 `Token`。
- 如果下游生产者需要前驱票面但没有 `reads`，让它显式加 `reads A`；不要让 `produces B` 自动赋予内容可见性。
- 读视图也要审计；否则 gate 过滤只是表面安全。

收益：关闭泄露，读 gate 真正有意义。代价：API 行为变化；agents 可能要多一次 read；需要明确“票面可见权 = 读权限”。

### 8.4 可发现性不变式

PACT 的结构只保证“前驱 token 在 Counter 上”，不保证“下游能拿到它的 ThingRef”：

- `produce` 不消费前驱，但也不返回前驱内容；下游如果只有 `produces B` 而没有读路径，就是盲生产。
- 下游要拿票面，通常得 `read` 前驱 token，或查 Counter/Archive 视图。
- 上游被消费后，票面只留在 Archive；需要 `depends_on` 追溯、保留期和访问权。B20 已说明没有 O(1) 祖先查询，跨 token 谱系要靠外部索引。

建议的可发现性不变式：

> 对任何被下游作为输入使用的 token，下游在行动时必须能获得其票面：或通过 read/视图，或票面已复制到下游 token，或存在一枚见证 token 承载它；若上游已消费，Archive 的保留期与访问权必须覆盖下游的最晚行动时间。

落地方式是**域级 manifest 声明 + lint**，不是改引擎：

- 每个非源类型声明它的 ref 供给方式：`readable-by` / `copied-to` / `witnessed-by` / `not-needed`。
- lint 检查：下游类型有该前驱，但没有任何供给路径 → 报“盲交接”。
- 这属于工具层/治理层，不进母本语义。

### 8.5 新项目还是留在项目内

建议 **分开，但不是全分**：

```text
PACT 仓库（保持小内核）
  ├─ pact-core / parser / analysis / engine / store   （不变）
  ├─ pact-ref-spec（纯）  URI/Identifier 校验 + RefBindings + ContractCard + Tombstone schema
  ├─ pact ref lint（离线 CLI）  读 .pact + *.refs.yaml，不碰引擎
  └─ spec/ref/v1/  URI profile、JSON Schema、resolver OpenAPI

独立仓库 / 服务（治理平面）
  ├─ pact-refd（resolve / invoke / verify / revoke）
  ├─ scheme adapters（https / git / oci / mcp / file…）
  ├─ registry / discovery（可选）
  ├─ auth / revocation / audit sidecar
  └─ conformance tests
```

判断标准：

| 条件 | 建议 |
|---|---|
| 只做文件/URI/静态引用，团队小 | 留在项目内，但放在独立 crate，严禁 core/engine 依赖它 |
| 需要网络解析、MCP 调用、注册表、多租户 ACL、审计、撤销 | 另起项目/服务 |
| 需要多种 scheme、多个外部系统连接器 | 另起项目，治理平面有独立发布节奏 |
| 需要保持 `dist/` 的单二进制/小依赖形态 | 另起项目，不要把 HTTP/OAuth/DB 依赖塞进 PACT |

倾向“另起项目”，因为治理平面和 PACT 的**威胁模型、依赖、发布节奏、运维形态**都不同。

### 8.6 另起项目时的边界

最重要的边界：

> **PACT daemon 永不调用治理平面；治理平面也永不直接写 PACT 状态。参与者是唯一的桥。**

```text
participants
   ├── PACT REST/MCP ──► pactd（Counter、token、ThingRef 字符串）
   │
   └── resolve/invoke ──► 治理平面（registry、resolver、contract、auth、audit）
                              │
                              └── 证据/补偿 ──► pactd（正常 seed/produce，不直写存储）
```

具体边界：

| 边界 | PACT 侧 | 治理侧 | 契约 |
|---|---|---|---|
| 线上字段 | `thing_ref` 是 URI/Identifier 字符串或 `null` | 解析引用、匹配 profile | 只传字符串，不传结构体 |
| 作者声明 | `.pact` 不变；旁车 `*.refs.yaml` | 定义并校验 RefBindings / ContractCard schema | 按 `(domain, type, scheme)` 匹配 |
| 运行时 | `seed / produce / consume` | `resolve / invoke / verify / revoke` | 参与者调用，引擎不调用 |
| 证据 | witness token | 解析成功后由参与者正常 seed/produce | 不用直写事件日志 |
| 审计 | 事件日志 / Archive / 读审计 | 访问与调用审计 | 用 `token_id` / `request_id` / `trace_id` 关联 |
| 身份 | 接口层解析 participant | 同一 principal / audience | OIDC 或 token exchange；不把凭据放进 ThingRef |
| 失败 | 消费成功与外部动作成功独立 | 返回可重试/永久错误 | 参与者决定重试或补偿 |
| 版本 | 事件 schema / API | 治理 API / 契约 schema / scheme 版本 | 兼容矩阵，单向依赖 |
| 依赖 | 不依赖治理 crate | 可依赖 wire 类型，不依赖 engine | 严禁反向依赖 |
| 部署 | `pactd` 单权威 | 独立服务或参与者内嵌 | 引擎不可用时不阻塞协调，治理不可用时可显式失败 |

最小 API 可以是：

```text
resolve(domain, type, ref, actor, intent) -> {
  handle | bytes | stream,
  digest, media_type,
  contract_id, version,
  expires_at
}

invoke(domain, type, ref, actor, operation, input, idempotency_key) -> {
  status,
  result_ref?, result_digest?,
  error?
}

describe(ref) -> ContractCard
revoke(ref, actor, reason) -> {}
```

注意：`resolve/invoke` 的 `domain/type` 是给治理侧选 profile 用的，不是让治理侧理解 PACT 语义。PACT 仍然只认类型和 token。

---

## 9. 新项目结构与 GitHub 管理

### 9.1 仓库拓扑：两个活跃仓库 + 一个可选拆分

推荐：

| 仓库 | 职责 | 依赖方向 |
|---|---|---|
| `pact`（现有） | 参考实现 + 母本/文档 + `pact-ref-spec`（纯） + `spec/ref/v1/` | 不依赖治理平面 |
| `pact-ref`（新建） | 治理服务、适配器、SDK、注册表、审计、一致性测试 | 依赖 `pact-ref-spec`；引导阶段为 `version + vendored 只读快照`，规范发布后改为纯版本 |
| `pact-ref-conformance`（可选，后期） | 语言无关一致性测试，给第三方实现 | 依赖 `spec/ref/v1/` |

为什么不一开始就拆很多仓库：

- 每种 scheme 一个仓库 → 发布矩阵爆炸，跨仓库版本漂移。
- 一个仓库包打天下 → 依赖膨胀，CI 变慢，安全面变大。
- 两个活跃仓库 + workspace 内多 crate，是当前阶段最省管理成本的形态。

`pact-dsh` 保持现有独立仓库，不受本专题影响。

### 9.2 `pact` 仓库要新增的部分

```text
pact/
├─ crates/
│  └─ pact-ref-spec/          # 纯：URI/Identifier、RefBindings、ContractCard、Tombstone、匹配
├─ spec/ref/v1/
│  ├─ uri-profile.md
│  ├─ ref-bindings.schema.json
│  ├─ contract-card.schema.json
│  └─ reference-plane.openapi.yaml
├─ crates/pact-cli/            # pact ref lint
└─ docs/thingref-governance.md # 本文件
```

`pact-ref-spec` 的约束：

- 纯函数、无 IO、无网络、无文件系统。
- 可编译到 `wasm32`（已实测）。
- 不依赖 `pact-engine`、`pact-store`。
- 只定义数据形状、校验、匹配；不定义解析/调用实现。

**落地状态（2026-10-27）**：crate 与 `spec/ref/v1/` 五份文件已创建；`pact-ref` 已通过 `ref-core` re-export 切换消费，采用
`version = "0.1.0"` + `vendor/pact-ref-spec` 只读快照，`scripts/sync-spec.sh` 负责同步与漂移检查。
`pact ref lint` 已实现（FR27/D41）：离线读取 RefBindings、ContractCard 与 `.pact` 声明，检查解析/规范校验、
类型对照、契约引用、scheme 一致性、策略告警与契约状态；不进引擎、不访问网络。
Counter/Archive 可见性已实现（FR28/D42）：daemon `[ref_bindings]` 加载同一份 RefBindings，默认 `read-gate`，
`domain`/`hidden` 显式覆盖，token 存在性不变。视图级读审计已实现（D43）：`read_audit = true` 时按视图/类型记录暴露计数。
保留与 tombstone 已实现（FR29/D44）：`lifecycle.retention` 与 `deletionPolicy` 收紧校验，
新增 `Tombstone` 记录 schema 与 `pact ref lint --tombstones` 交叉检查；不做计时或自动删除。
可发现性 lint 已实现（D44 收尾）：对 `required=true` 检查 hidden、视图读取路径、Rule-R/consume 路径、
保留证据与删除可区分性，共 5 个 warning code。
`pact-ref` 的 vendored 快照已同步至 D44 版本，`scripts/sync-spec.sh --check` 通过，其 workspace 44 项测试通过。
推送前安全审视修复了 tombstone 时间戳的非 ASCII 边界 panic（已加回归测试）。
落地提交：`pact` `c5a9cd2`（静态层、可见性、视图审计、保留证据），`pact-ref` `2dbf440`（快照同步）；
修复与审视记录见 `docs/design-change.md` 第 72 轮。
可发现性 lint 落地与推送前安全审视：`pact` `eb86654`（2026-10-28），`pact-ref` 无新改动；
`scripts/sync-spec.sh --check` 通过。记录见 `docs/design-change.md` 第 74–75 轮。
`pact-ref` 远端：<https://github.com/onlookor/pact-ref>（公开）。
`refd` 的 resolve/describe 端点已按 §13.6 在 `pact-ref` 实现（2026-10-31）；下一步是动态只读阶段。

### 9.3 `pact-ref` 仓库结构

```text
pact-ref/
├─ README.md
├─ CONTRIBUTING.md
├─ SECURITY.md
├─ GOVERNANCE.md
├─ Cargo.toml                 # workspace
├─ Cargo.lock
├─ rust-toolchain.toml
├─ SPEC_VERSION               # 引导阶段记录的预期 spec 版本
├─ .github/
│  ├─ CODEOWNERS
│  ├─ workflows/
│  │  ├─ ci.yml               # fmt / clippy / test / build / docs
│  │  ├─ release.yml
│  │  └─ security.yml         # cargo audit + dependency review
│  ├─ ISSUE_TEMPLATE/
│  │  ├─ bug.yml
│  │  ├─ scheme-proposal.yml
│  │  ├─ contract-card.yml
│  │  └─ security.yml
│  └─ pull_request_template.md
├─ crates/
│  ├─ ref-core/               # 治理内部：策略选择、卡片求值、解析器路由；依赖 pact-ref-spec
│  ├─ ref-resolver/           # resolve SPI + 适配器注册表
│  ├─ ref-invoker/            # invoke SPI + 动态能力注册表
│  ├─ ref-registry/           # 可选发现
│  ├─ ref-auth/               # authn/authz、audience、撤销
│  ├─ ref-audit/              # 审计边车
│  └─ ref-client/             # 参与者 SDK
├─ services/
│  └─ refd/                   # 服务二进制
├─ adapters/
│  ├─ http/
│  ├─ git/
│  ├─ oci/
│  ├─ file/
│  └─ mcp/
├─ conformance/
│  ├─ cases/
│  └─ runner/
├─ examples/
│  ├─ bindings/
│  ├─ contracts/
│  └─ mcp/
├─ deploy/
│  ├─ docker/
│  └─ systemd/
└─ docs/
   ├─ architecture.md
   ├─ adr/
   ├─ schemes/
   └─ operations.md
```

引导阶段的已知差异：`conformance` 与 `docs` 作业暂并入 `ci.yml`，等一致性用例落地后再拆独立 workflow。
`ref-core` 已改为依赖 vendored `pact-ref-spec` 并 re-export 规范类型；`SPEC_VERSION` 记录为 `0.1.0-vendored`，
规范发布到注册表后改为纯版本依赖，并同步该文件与快照删除。

### 9.4 GitHub 管理约定

| 项 | 约定 |
|---|---|
| 组织与团队 | 一个组织下两个活跃仓库；`@org/pact-core` 管 `pact`，`@org/pact-ref` 管新仓库，`@org/security` 跨仓库安全审阅 |
| CODEOWNERS | spec 变更必须 `pact-core` + `pact-ref` 双方审阅；服务/适配器由 `pact-ref` 审阅；安全文件由 `security` 审阅 |
| Issue 模板 | bug、scheme proposal、contract card proposal、security 四类；security 走私有报告，不进公开 issue |
| PR 模板 | 必填：是否改 spec、是否改 contract schema、兼容性影响、测试、文档、安全影响 |
| 项目板 | 列：proposal → spec → implementation → conformance → release；用 labels 标记 `area/spec`、`area/resolver`、`area/mcp`、`area/security`、`compat` |
| 里程碑 | `v0.1 static`、`v0.2 dynamic`、`v1.0 spec`；每个里程碑只承诺一个可验收能力 |
| CI | fmt、clippy、test、`cargo deny`、`cargo audit`、schema 校验、conformance、docs link check、secret scan、容器构建、SBOM |
| 发布 | `pact-ref-spec` 用语义化版本；`pact-refd` / `ref-client` 独立版本；发布 GitHub Release + checksums + SBOM |
| 分支保护 | `main` 必须过 CI；线性历史；禁止 force push；发布标签保护 |
| 跨仓库变更 | 先在 `pact` 提 spec PR 并发布 `pact-ref-spec vX.Y`，再在 `pact-ref` 提实现 PR 并固定版本；不做跨仓库原子合并 |
| 依赖共享 | 不用 git submodule；用发布 crate + JSON Schema/OpenAPI 发布产物；`SPEC_VERSION` 文件记录消费版本 |
| 文档 | spec 用 GitHub Pages 发布版本化文档；ADR 记录“为什么”；操作手册放 `docs/operations.md` |
| 安全 | `SECURITY.md`、Dependabot、CodeQL、secret scanning、SSRF/路径穿越测试、发布签名；ThingRef 永不含凭据 |

### 9.5 为什么这样便于 GitHub 管理

- **两个活跃仓库**：权限、CODEOWNERS、项目板、发布列车都简单；不会出现十几个适配器仓库的版本矩阵。
- **spec 与实现分离**：`pact` 拥有规范与纯 schema，`pact-ref` 拥有实现；跨仓库变更用版本标签衔接，避免 submodule 和跨仓库原子合并。
- **workspace 内多 crate**：适配器、服务、SDK 可以独立发布，但共享 CI 和依赖锁，降低漂移。
- **可选一致性仓库**：第三方实现出现后再拆 `pact-ref-conformance`，不提前制造维护负担。
- **安全边界清晰**：治理平面的网络/凭据/审计依赖不进 `pact` 核心，`pact` 的单二进制和小依赖形态得以保留。

---

## 10. 决策记录

以下 1–8 对应“ThingRef 治理开放问题”。2026-10-26 裁决。

### 议题 1：票面形态

**决策：URI-first，Identifier-legacy，`∅` 保留；不强制 URI，不破坏 token/事件/日志兼容。**

收益：

- 新引用有 scheme，解析器选择确定。
- 旧 `REQ-001` 等 Identifier 仍可用，避免大规模迁移。
- 不需要改母本 §3.2。
- 统一校验、lint、scheme 白名单、secret 扫描仍可逐步推进。

代价：

- 未分类 Identifier 无法直接解析，需要上下文或注册表。
- lint 要区分“URI”和“未分类引用”，不能一刀切。
- 短 ID 仍然可能被误用为地址。

缓解：

- 新引用默认 URI；Identifier 标记“未分类”，lint 提示。
- 归档/跨边界交付优先要求 URI 或 digest。
- 不 bump 事件 schema；只在入口做可选校验。

### 议题 2：`∅` 与“未提供”

**决策：区分发生在入口，token 两态。**

收益：

- 抓“忘了传 ref”的作者错误。
- token/事件/CPN/消费者不引入 tri-state。
- `required` 可以在入口强制执行。

代价：

- API 客户端要显式传字段；旧客户端可能失败（这是入口契约变化，不是事件不兼容）。
- 需要在入口层增加校验逻辑。

缓解：

- `required: false` 的类型允许 `null`；只有 `required: true` 才拒绝。
- 错误信息明确区分“缺失”和“确认无制品”。

### 议题 3：契约卡位置

**决策：域旁 sidecar 为权威，注册表只做可选发现。**

收益：

- `.pact` 保持纯；契约独立演进；随域版本发布。
- 不改 parser/engine。
- ContractCard 可跨域复用。

代价：

- 两份文件要同步；部署要带；需要 lint。
- 没有注册表时，跨域发现较弱。

缓解：

- `RefBindings` 随域 bundle 发布；`declarationDigest` 防止漂移。
- `pact ref lint` 校验引用完整性。
- 注册表可选，不做强制全局中心。

### 议题 4：Archive 保留期

**决策：ContractCard 声明内容保留；PACT 保证协调保留；删除发 tombstone。**

收益：

- 审计诚实：不会出现“PACT 说归档了，制品却消失”。
- 内容保留与协调保留分开，法律删除可解释。
- tombstone 让消费者显式失败。

代价：

- 两套 retention 要协调。
- tombstone 是额外 token 类型，参与者要处理。
- 删除与审计保留可能冲突。

缓解：

- 进入 Archive 的 ref，内容保留期 ≥ 下游最晚行动时间。
- 日志压缩只影响事件历史，不改变 Archive 协调记录；需要完整历史时导出审计包。
- tombstone 通过正常 seed/produce 产生，不直写存储。

### 议题 5：Counter/Archive 的 `thing_ref` 可见性

**决策：默认 `read-gate`，显式 `domain` 才公开，`hidden` 给敏感引用。**

收益：

- 关闭“域级授权即可看票面”的泄露。
- 读 gate 真正有意义；与 Rule-R 语义一致。
- 静态 gate 判断，性能可控。

代价：

- API 行为变化；agents 可能多一次 read。
- 视图要带 participant/principal。
- 生产者若需要前驱票面，必须显式有 `reads`。

缓解：

- 视图返回 token 存在性，只过滤 `thing_ref`。
- 读视图也审计。
- 提供 `domain` / `hidden` 的显式 profile 覆盖。

### 议题 6：复合制品

**决策：manifest 承载“包”，独立 token 承载“需要独立准入/消费的部分”。**

收益：

- 一次交接、一个契约、整体版本。
- 局部需要独立 gate 时不被牺牲。
- 不依赖原子多输出（B15/X3）。

代价：

- manifest schema/解析器。
- 包与部分可能不一致，需要校验。
- 可能出现重复（manifest + 部分 token）。

缓解：

- manifest 只列稳定 ref 和元数据，不复制内容。
- 需要独立动作的部分才发 token；其余留在一个包里。
- lint 检查 manifest 成员与 token 的一致性。

### 议题 7：动态调用审计边车

**决策：可选，放在治理平面或 `pact-runtime`，绝不放进 engine/core。**

收益：

- 动态调用可追责、可解释重放缺口。
- 不污染 PACT 事件日志。
- 可以按 retention/隐私策略独立治理。

代价：

- 新存储/隐私/保留策略。
- 与 PACT 日志的关联要设计好，否则只是多一份日志。

缓解：

- 最小记录只含 ID、摘要、状态、时间；不记录正文/凭据/个人数据。
- 默认关闭；开启时明确 retention。
- 用 `token_id` / `request_id` / `trace_id` 关联。

### 议题 8：MCP 最小 profile

**决策：两档，而不是一个最小 profile。**

收益：

- 低风险只读工具低门槛。
- 高风险 mutating 工具有完整保证。
- 避免“四个字段看起来够了”的误判。

代价：

- 两套 profile 要维护。
- 工具作者要正确分类。
- 分类错误比没有分类更危险。

缓解：

- `mcp-read` 限定 read-only、幂等、结果可物化。
- `mcp-mutating` 强制完整字段。
- MCP hints 只作提示，契约卡必须显式声明。
- 一致性测试覆盖两类。

---

## 11. 契约卡示例

### 11.1 RefBindings 示例

```yaml
# se_lifecycle.refs.yaml —— 与 .pact 同目录、同版本发布；引擎不读取
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: se_lifecycle
  declarationDigest: sha256:...
spec:
  defaults:
    ref:
      required: true
      visibility: read-gate
      pinning: required-at-archive
      schemes: [urn, https, git, oci]
      onUnknownScheme: reject
  types:
    requirement:
      ref:
        schemes: [urn, https]
        contract: se/requirement@1
    design:
      ref:
        schemes: [https, git]
        contract: se/design-review@2
    implementation:
      ref:
        schemes: [git, oci]
        contract: se/implementation@1
    test_report:
      ref:
        schemes: [https]
        contract: se/test-report@1
    deployment:
      ref:
        schemes: [oci]
        contract: se/deployment@1
    review_meeting:
      ref:
        required: false
        value: none
```

### 11.2 静态制品契约卡

```yaml
apiVersion: pact.ref/v1
kind: ContractCard

metadata:
  id: se/design-review
  version: "2"
  owner: architecture-guild
  status: active
  labels: [design, review, internal]

spec:
  artifact:
    kind: document
    mediaType: text/markdown; charset=utf-8
    schema: https://schemas.example.com/se/design-review/v2.json
    schemaDigest: sha256:6f1c...
    digestPolicy: required-at-archive

  reference:
    schemes: [https, git]
    uriPattern: "https://artifacts.example.com/design/{id}#sha256=<hex>"
    pinning:
      requiredForArchive: true
      accepted: [sha256, git-commit]
    mutableAllowed: false
    maxLength: 2048

  parties:
    producer: architect
    custodian: artifacts-team
    consumers: [developer, tester, release-manager]
    consumerScope: "domain:se_lifecycle"

  resolution:
    mode: fetch
    endpoint: https://artifacts.example.com
    auth:
      scheme: oauth2
      audience: pact-ref
      scopes: [artifact:read]
      credentialSource: broker      # 凭据不写进 ThingRef
    timeoutMs: 5000
    retry: none

  lifecycle:
    retention: "P365D"
    deletionPolicy: tombstone-then-delete
    tombstoneRef: "urn:pact:tombstone:se/design-review"

  failure:
    notFound: permanent
    unauthorized: permanent
    timeout: retryable
    digestMismatch: permanent
    onFailure: emit-witness-token    # 参与者用 PACT 正常 seed/produce 记录

  verification:
    conformance:
      - schema-validation
      - digest-check
      - media-type-check
    suite: https://github.com/example/pact-ref-conformance/se-design-review
```

### 11.3 动态 MCP 能力卡

```yaml
apiVersion: pact.ref/v1
kind: ContractCard

metadata:
  id: build/mcp-build
  version: "3"
  owner: build-platform
  status: active

spec:
  capability:
    kind: mcp-tool
    server:
      id: mcp://build.example.com/build-server
      trustAnchor: https://build.example.com/.well-known/mcp
      protocol: "2025-06-18"       # 示例；按实际 MCP 版本
    tool:
      name: build.run
      version: "3"
      inputSchemaDigest: sha256:...
      outputSchemaDigest: sha256:...

    sideEffect: mutating
    idempotency:
      key: "token_id + request_id"
      semantics: at-most-once

    result:
      materialization: required
      returns: urn:pact:artifact:build-result
      recordDigest: true

    auth:
      audience: pact-ref
      scopes: [build:run]
      credentialSource: broker

    timeoutMs: 600000
    failure:
      timeout: retryable-with-idempotency-key
      buildFailed: permanent
      unauthorized: permanent

  reference:
    schemes: [mcp]
    uriPattern: "mcp://build.example.com/build-server/build.run@3#schema=sha256:..."
```

### 11.4 契约卡最小字段

| 字段 | 回答的问题 |
|---|---|
| `metadata.id` / `version` / `owner` | 这是什么、第几版、谁负责 |
| `artifact.kind` / `mediaType` / `schema` / `digestPolicy` | 下游拿到的是什么 |
| `capability.server` / `tool` / `sideEffect` | 动态能力的服务方、操作、副作用 |
| `reference.schemes` / `uriPattern` / `pinning` | 票面允许什么、如何定版 |
| `parties` / `consumerScope` | 谁生产、谁保管、谁可消费 |
| `resolution` / `invocation` / `auth` | 怎么取、怎么调、凭什么 |
| `lifecycle` / `retention` / `deletionPolicy` | 过期、删除、保留 |
| `failure` / `onFailure` | 失败算谁的、如何显式处理 |
| `verification` / `conformance` | 怎么验收 |

这些字段不是越多越好，而是**每个字段都对应一个下游会问的问题**。

---

## 12. 反模式与驳回

以下方案明确驳回：

- 引擎侧 fetch / dereference / 存在性校验 / 自动重试。
- 把制品内容、二进制、大 JSON 塞进 ThingRef。
- 把密钥、bearer token、签名私钥塞进 ThingRef。
- 让 Rule-P 或 GateExpr 读 ThingRef 做内容/版本判定（X4）。
- 动态 ref 不物化就直接进 Archive 作为终态。
- 强制全局唯一注册表或“一个 scheme 统一所有制品”。
- 用 ThingRef 做路由键/匹配键。
- 把开放契约写进 `.pact` 语法；只有有限分类才进 TypeName。
- 把 `(domain, type) → contract` 做成唯一全局映射。

### 12.1 预期抗辩与回应

| 抗辩 | 回应 |
|---|---|
| “这会变成制品管理系统” | 契约、解析器、注册表都在范式外；只保留六条边界；任何需要引擎读 ref 的规则不采纳 |
| “URI 已经够了” | 语法层够了，治理层不够；URI 不解决契约、信任、时效 |
| “统一注册表最省心” | 强制统一会引入单点、耦合、治理瓶颈；默认自描述 + 可选注册表 |
| “动态调用是未来” | 动态调用有副作用、不可重放、会漂移；静态 ref 是审计锚点 |
| “所有 ref 都必须钉死版本” | 工作态可 mutable；归档/跨边界必须 pin |
| “默契就够了” | 同团队短周期可以；跨组织/长周期/自动化必须升级 |
| “让引擎顺手解析” | 破坏零外部副作用、单权威、可重放；还会引入 SSRF/凭据/网络故障 |
| “示例里 REQ-001 已经够用” | 那只证明同团队默契；跨团队时既不是地址也不是契约 |

---

## 13. 落地顺序与验收

### 13.1 顺序

0. **仓库引导（已完成，2026-10-26）**：`pact-ref` 仓库已初始化，首个提交为 `5895bef`；
   workspace、7 个纯骨架 crate、5 个 adapter 占位、`refd` 引导二进制、docs/examples/deploy、
   GitHub CI/security/release 均已就绪；`cargo fmt/clippy/test` 全绿。
1. **静态阶段（已完成，2026-10-27）**：`pact-ref-spec` 纯 crate + `spec/ref/v1/`（URI profile、两份 schema、OpenAPI）已落地，
   18 项单测与 wasm32 构建通过；`pact-ref` 已切换消费（当前 46 项测试通过）；`pact ref lint` 已实现（FR27/D41），
   覆盖绑定/契约/声明一致性与退出码。
   **已完成（2026-10-31）**：`refd` 的 resolve/describe 端点（契约 §13.6）——`deny`/`static`/`oidc` 认证、端点 scope + 对象 ACL、审计 fail-closed、`file`/`https` adapter；OIDC（FR35/D51）支持 JWKS 文件/HTTPS、算法白名单、`iss`/`aud`/`exp`/`nbf`、未知 `kid` 限频刷新，失败统一 `401`。
   **已完成（2026-10-28）**：REST 写入路径入口校验（FR30/D45）——`POST /seeds`、`POST /tokens` 区分缺失/`null`/字符串，执行 URI profile 与 RefBindings 策略，错误码 `ThingRefMissing`/`ThingRefRequired`/`InvalidThingRef`；CLI/MCP 经 REST 继承。
   **已完成（2026-10-29）**：`value`、`onUnknownScheme` 与钉版运行期强制（FR31/D46）——`pinning: required` 写入即拦，`required-at-archive` 在 consume 进入 Archive 前拦，错误码 `ThingRefUnpinned`。
   **已完成（2026-10-30）**：ContractCard 运行期加载与版本不可变（FR32/D47）——`[ref_contracts]` 启动加载、`contract_id()` 索引、按数据触发闭环、重复 `id@version` 拒绝、consume 钉版并集（只加严）；卡片与绑定显式重载（FR33/D48）——`POST /ref/reload` 与 `pact ref reload`、同事务原子换快照、`dryRun` 影响报告、审计与 ACL 覆盖。
   **已完成（2026-10-31）**：`declarationDigest` v1（FR34/D49 第一步）——`pact ref digest` 工具、`pact ref lint` 漂移比对（缺失 warning / 不一致 error）、规范 `spec/ref/v1/declaration-digest.md` 与 golden 向量；同日完成第二步——`[ref_digest] mode = off/warn/enforce`（默认 `warn`）在 `POST /domains` 进入引擎前比对，`enforce` 不一致返回 `400 DeclarationDigestMismatch`，启动恢复与 `ref reload` 不比对。
   **已完成（2026-10-31）**：`pact-wasm` 本地宿主写入侧入口校验（FR30/D50）——把 REST 的入口语义抽为 `pact-ref-spec::validate_entry` 纯函数，REST 与 WASM 共用；WASM 由宿主以 JSON 注入 RefBindings（`refBindings` / `PACT_WASM_REF_BINDINGS`），未注入时只做结构规则；省略与 `null` 均表示 `∅`。ContractCard 与 consume 前归档钉版仍只在 REST 边界。
2. **可见性阶段（已完成，2026-10-27）**：Counter/Archive 返回前过滤 `thing_ref`（FR28/D42）；
   默认 `read-gate`（无条件 `reads`/`consumes`），RefBindings 的 `domain`/`hidden` 显式覆盖；
   daemon `[ref_bindings].paths` 加载 profile，失败 fail-closed；令牌/签名模式下视图要求身份。
   **已完成**：视图级读审计（D43）——`read_audit = true` 时同一审计流记录 `kind=view`，按视图/类型聚合 `tokens/refs_visible/refs_filtered`，不记票面值；写失败 fail-closed。
3. **保留阶段（已完成，2026-10-27）**：ContractCard `lifecycle.retention` / `deletionPolicy` / `tombstoneRef`
   已在规范层收紧校验（FR29/D44）；新增 `Tombstone` 记录 schema 与 `pact ref lint --tombstones` 交叉检查。
   **已完成**：可发现性 lint——对 `required=true` 检查 hidden、视图读取路径、Rule-R/consume 路径、保留证据与删除可区分性（5 个 warning code，`--strict` 视为失败）。
4. **动态只读阶段（已完成，2026-10-31）**：`mcp-read` profile 校验、`refd` 的 `POST /v1/invoke`、`mcp` adapter（JSON-RPC `tools/call`）与内容寻址物化（FR36/D52）；结果经 `file://` 引用回到 `/v1/resolve`，参与者以正常 `seed`/`produce` 产生 witness token。动态写阶段与 `revoke` 仍未开始。
5. **动态写阶段（规范层已实现，2026-10-31；端点待做）**：`pact-ref-spec` 已强制 `mcp-mutating` 卡片级必填字段（§13.8.2）；`POST /v1/mutate`、幂等台账、两阶段审计与 witness/compensation 边界待实现（FR37/D53，§13.8）。
6. **可选注册表**：只做发现，不做真相源。

### 13.2 测试

- **单元测试**：URI/Identifier 解析、RefBindings 匹配优先级、ContractCard 校验、retention/tombstone 校验、`∅`/缺失区分。
- **集成测试**：入口校验、Counter/Archive 可见性过滤、`thing_ref` 不进入 gate、witness token 证据链。
- **端到端测试**：静态制品 resolve → witness token → 下游生产；动态 `mcp-read` → 物化 → 新 token；tombstone 交叉检查。
- **回归测试**：旧 Identifier 仍可写入/读取；`ThingRef` 不透明性不变；CPN 颜色集不变。
- **安全测试**：SSRF、路径穿越、凭据泄露、日志注入、超长 URI、危险 scheme、URI userinfo。
- **WASM 测试**：静态校验可用（`prepare_seed` / `prepare_produce` 入口校验、绑定注入与规范化）；无网络依赖；AOT/体积门禁不退化。
- **一致性测试**：ContractCard 两种 profile 的 conformance cases。

### 13.3 验收

- `.pact` 文法零改动；parser/domain 不认识 ThingRef。
- `pact-core`/`pact-engine` 不依赖治理服务、网络适配器或 IO 实现。
- 引擎无 fetch/invoke/dereference。
- 旧 Identifier 仍可工作；新引用默认 URI。
- `thing_ref` 默认不向无读权限的视图暴露。
- ContractCard/RefBindings/Tombstone 有机器可校验 schema；归档钉版契约的删除策略必须有 tombstone 路径。
- 要求引用（`required=true`）的类型必须通过可发现性静态检查：存在视图或 Rule-R/consume 读取路径，且契约声明保留期；`delete` 策略需有 tombstone。
- 动态 ref 不能作为 Archive 终态；必须有物化路径。
- 治理平面不在 PACT daemon 进程中运行。

### 13.4 待决方案分析（2026-10-29）

两项治理缺口先以“选项、收益与代价”定案：B0（卡片加载，`FR32`/`D47`）已实现；`declarationDigest` 已于 2026-10-31 确认并完成两步（`FR34`/`D49`：工具/lint 与运行期 `[ref_digest]` 比对）。

#### 13.4.1 ContractCard 是否在运行期加载（B0 已实现，2026-10-30）

**现状（2026-10-30 前）**：RefBindings 的有效规则里有 `contract`（如 `se/design@1`），`pact ref lint` 会检查卡片存在性与生命周期一致性；但 daemon 只加载 `[ref_bindings]`，运行期完全不读 ContractCard。于是卡片里的 `reference.pinning.requiredForArchive`、`artifact.digestPolicy: required-at-archive` 只影响 lint，不影响 consume。

**现状（2026-10-30 起）**：daemon 已在启动时加载 `[ref_contracts]`，按 `contract_id()` 建索引；绑定声明 `contract` 时缺卡即拒绝启动；consume 的有效钉版要求为“绑定 ∪ 卡片”。实现见 `FR32` / `D47` 与 `docs/design.md` §9.14。

| 选项 | 收益 | 代价 |
|---|---|---|
| **A. 保持只读于 lint** | 零新配置、零迁移、运行期无额外 IO；实现面最小 | `contract` 字段在运行期等同注释；卡片与绑定成为双源，改一处忘另一处；使用者容易误以为归档钉版已由卡片强制 |
| **B. 卡片必配并强制** | 契约成为唯一权威；lint 与运行期闭环；能承载后续 `artifact.kind`/`mediaType` 检查 | 新配置面与启动校验；既有“声明了 contract 但没挂卡片”的部署会启动失败；需定义卡片与绑定冲突时的优先级；卡片文件漂移/热加载语义；测试矩阵明显扩大 |
| **C. 按数据触发加载 + 启用即 fail-closed + 只加严**（推荐） | 声明了 `contract` 的部署获得与 B 相同的闭环（缺卡即启动失败）；未声明 `contract` 的部署零配置、零迁移；卡片只能“加严”不能放宽，避免优先级歧义 | 未声明契约的部署没有“卡片强制”的全局保证（但那里不存在可强制的卡片）；需要文档与 lint 提示；配置后重启才能加载卡片 |

**推荐 C 的具体语义**：

1. daemon 配置新增可选 `[ref_contracts].paths`，目录展开规则与 `[ref_bindings]` 一致；启动时加载并 `ContractCard::validate`，按 `contract_id()`（即 `metadata.id@version`）建索引，与绑定的 `contract` 引用键一致。
2. **按数据触发闭环**：只要任一 RefBindings 的有效规则（合并继承后）声明了 `contract`，就必须配置 `[ref_contracts]` 且该 contract 能解析，否则**启动失败**（fail-closed）；完全没有 `contract` 引用的部署无需配置，行为不变。这样 C 在“实际使用契约”的部署上等价于 B 的闭环，但不惩罚未使用契约的部署。
3. 有效钉版要求 = 绑定 `pinning` **或** 卡片 `reference.pinning.requiredForArchive` / `artifact.digestPolicy: required-at-archive`（单调并集，卡片不放宽绑定）。
4. 首版只在启动时加载，不支持热更新；卡片变更需重启。这是**首版的激活机制**，不是终身约束；B1 已提供显式重载（§13.5），卡片仍按 `id@version` 不可变。
5. 不把卡片校验塞进引擎；仍在 REST 边界（consume 前）。

**C 是否保留 B 的闭环与可扩展收益**：

- **闭环**：在声明了 `contract` 的部署上完全保留——缺卡即启动失败，运行期必定读到卡片；仅对“从未引用 contract”的部署不启用，而那里本来没有可强制的卡片。B 多出来的只是“即使没有 contract 引用也要求挂卡”的形式统一，没有治理收益，只增加迁移成本。
- **可扩展**：卡片加载与索引就位后，`artifact.kind` / `mediaType`、生命周期等检查与 B 使用同一份数据，差异只在配置门槛，不在能力上限；这些字段检查首版未启用（FR32 未生效项）。
- **唯一代价**：从未声明契约的部署仍没有“卡片强制”的全局保证；这由第 2 条的“按数据触发”把范围收窄到“没有可强制对象”的情况，并应配 lint 提示（声明了 `contract` 即视为启用）。

**已确认（2026-10-29）**：采纳“按数据触发闭环”的 C（声明 contract ⇒ 必须挂卡且解析，否则启动失败；未声明 contract ⇒ 无需配置）；首版接受重启加载，作为激活机制而非终身约束。实现编号为 `FR32` / `D47`（B0 加载与不可变）；显式重载见 §13.5 的 B1（`FR33` / `D48`）。

#### 13.4.2 `declarationDigest` 的摘要对象与规范化（已确认 2026-10-31；两步均已实现）

**现状（2026-10-31 起）**：`pact ref digest` 可按 v1 规范计算摘要，`pact ref lint` 在能读到 `.pact` 时计算并比对绑定摘要——缺失为 warning `ref.bindings.digest_missing`，已记录且不一致为 error `ref.bindings.digest_mismatch`。运行期 `POST /domains` 按 `[ref_digest] mode`（默认 `warn`）比对；`enforce` 不一致返回 `400 DeclarationDigestMismatch`，启动恢复与 `ref reload` 不比对（D49 第二步已实现）。

| 选项 | 收益 | 代价 |
|---|---|---|
| **A. 声明原文字节** | 实现最简单；可离线 `sha256sum`；无规范化规范负担 | 对空白、注释、换行风格、字段顺序敏感；REST 客户端重排版即误报；治理上把“格式”当“语义” |
| **B. 规范化 AST 序列化** | 对格式稳定；可表达“同一声明”的等价性 | 必须定义并版本化 canonical form；序列化器改动会改变全量摘要；跨语言/跨版本一致性成本高；误报排查困难 |
| **C. 语义接口摘要**（推荐） | 只覆盖治理关心的部分（类型、前置条件、gates、`seed_authority`），排序固定；注释/格式变化不影响；比 B 的规范化面小 | 仍需定义版本化规范；语义规范化（去重、排序）本身可能出现歧义；字段增减即摘要变化（这是期望行为，但需公告） |
| **D. 暂不实现/移除字段** | 无死字段、无误导 | 失去“绑定对应哪版声明”的证据；移除是 schema 破坏性变更；保留则是死字段 |

**推荐 C，并分两步**：

1. **先工具与 lint**：新增 `pact ref digest`（输出 `sha256:<64 hex>`），`pact ref lint` 在能读到 `.pact` 时计算并比对绑定中的摘要；规范里写清“摘要对象 = 语义接口、排序规则、参与字段”。
2. **后运行期**：daemon 在 `load_domain` 时对声明计算摘要，与该域 RefBindings 中声明的 `declarationDigest` 比对。模式由配置决定，首版默认不阻塞：

   | 模式 | 行为 |
   |---|---|
   | `off` | 只计算不比对，等同 A 的现状 |
   | `warn`（默认） | 不一致写结构化 `WARN`（`expected` / `actual` / 摘要算法版本），`load_domain` 照常成功 |
   | `enforce`（显式开启） | 仅对“声明了非空 `declarationDigest`”的域，不一致返回 `400 DeclarationDigestMismatch` |

   三条边界：绑定未写 `declarationDigest` ⇒ 任何模式都不阻塞；摘要算法版本不受运行时支持 ⇒ 按“不可比对”处理（`warn`），不当作“不一致”；**启动恢复与 `ref reload` 不比较**，`enforce` 仅作用于新的 `load_domain`，避免单域绑定笔误导致整个 daemon 无法启动（D49 第二步已按此实现）。

#### 13.4.3 摘要误算会不会直接阻塞域加载

**结论：按推荐方案不会默认阻塞；只有显式 `enforce` 且摘要存在但不匹配时才阻塞。**

先回答两项决策的耦合：**13.4.1 选 C 与 13.4.2 选 C 相互独立**。`[ref_contracts]` 是否配置、是否为空都不参与摘要比对；摘要比对只读 RefBindings 的 `metadata.declarationDigest` 与摘要模式。因此“可选 `[ref_contract]`”不会因为卡片缺失而顺带触发摘要校验，反之亦然。

若默认 `enforce`，则下列任一“误算”都会让域无法 `load_domain`，且故障发生在运行期（运维只能改绑定或降级配置）：

| 误算来源 | 是否真实漂移 | 默认 `enforce` 的后果 | 缓解 |
|---|---|---|---|
| 工具与 daemon 的规范化实现版本不同 | 否 | 域无法加载（假阳性） | 工具与 daemon 共用同一 `pact-ref-spec` 实现；摘要带算法版本；版本不支持只告警 |
| 语义规范化定义有歧义（如 gates 排序键、去重规则） | 否 | 域无法加载（假阳性） | 规范里冻结排序/去重规则；golden 用例与 conformance 锁定 |
| 手写或复制摘要 | 否 | 域无法加载（假阳性） | lint 强制“摘要只能由工具生成”；报告 `expected`/`actual` 便于比对 |
| 声明确实被改动 | 是 | 域无法加载（这正是期望） | 提交流程先跑 `pact ref digest` 更新绑定，再发布 |

所以顺序上先 `warn`：先让工具与运行时在同一实现上稳定，并用 lint 把“摘要是否最新”变成提交前检查；等 golden/conformance 覆盖稳定后再把 `enforce` 变成可选开关，最后才考虑默认开启。这样“真实漂移”在 lint 阶段就被拦，“误算”不会直接变成域不可加载。

**已确认（2026-10-31）**：摘要对象 C（语义接口）、两步走（工具/lint 先行）、运行期默认 `warn` 且 `enforce` 显式开启、字段保持 `sha256:<64 hex>`；编号为 `FR34` / `D49`，规范落点为 `spec/ref/v1/declaration-digest.md`，实现落点为 `pact-analysis::digest`、`pact-cli` 与 `pact-http`。两步均已实现；`[ref_digest].mode` 与 `DeclarationDigestMismatch` 已登记于 `docs/architecture.md` 附录 A（第 87 轮）。详见 §13.4.2.1。

**依赖关系**：13.4.1 与 13.4.2 相互独立；卡片加载已完成（D47/D48），摘要两步均已完成。`[ref_digest].mode` 与 `DeclarationDigestMismatch` 已先登记再实现。

#### 13.4.2.1 定义 v1（2026-10-31 已确认；两步均已实现）

**摘要对象（已确认 C：语义接口）**：只覆盖域声明中影响绑定匹配、可见性与准入判定的语义面；注释、空白、文件路径、字段书写顺序、实例名与分析结果都不参与。

| 字段 | 参与 | 理由 |
|---|---|---|
| 域名（`load_domain` 的 `name`；CLI 取 `--name`，缺省文件 stem） | 是 | 防止同一正文在不同域名下复用摘要；绑定以 `metadata.domain` 为键 |
| `types` | 是（排序去重） | 绑定按类型键匹配，类型集合是接口主轴 |
| `preconditions` | 是（rhs → 排序后的前驱集合） | 决定源类型与 Rule-P 前置，影响 `seed_authority` 的必要性 |
| `gates` | 是（按 `(participant, action, type)` 排序；含条件表达式） | read/consume 的条件决定视图暴露与归档路径，是 `visibility`/`pinning` 的作用面 |
| `seed_authority` | 是（按 `(participant, type)` 排序） | 决定谁可点火 |
| 注释、空白、换行/缩进、字段顺序、文件名、`.pact` 原文 | 否 | 格式变化不是语义漂移 |
| 实例名、分析结果、RefBindings/ContractCard 内容 | 否 | 不属于声明接口；卡片与绑定的生命周期独立 |

**规范化**：UTF-8 紧凑 JSON（无缩进、无尾换行），顶层固定为
`{"kind":"pact.declaration.interface","v":1,"domain":…,"types":[…],"preconditions":[…],"gates":[…],"seed":[…]}`；
类型/参与者用字符串；条件表达式用自定文法文本 `via(T)` / `exists(T)` / `count(T,>=,n)` / `and(A,B)` / `or(A,B)` / `not(A)`，不使用 Rust `serde` 派生枚举的表示（派生细节不是协议）。摘要 = `sha256:` + 上述字节的小写十六进制 SHA-256。完整字段顺序、排序规则与 golden 向量见 `spec/ref/v1/declaration-digest.md`。

**实现落点**：`pact-analysis` 新增纯函数 `declaration_digest(domain_name, &pact_core::Domain) -> String`（该 crate 已依赖 `pact-core`；新增 `serde_json` 与 `sha2` 为 workspace 既有依赖）；CLI 与 `pact-http` 共用，不进入 `pact-ref-spec`（后者保持零 `pact-core` 依赖，可编译到 wasm32）。

**启用方式（已确认两步）**：

1. **第一步（工具 + lint，已实现，不改运行期）**
   - 新增 `pact ref digest <file.pact> [--name <domain>] [--json]`，输出 `sha256:<hex>`（JSON 另含 `algorithm`/`digestVersion`）。
   - `pact ref lint --declaration` 时计算并与绑定 `metadata.declarationDigest` 比对：**不一致 → error `ref.bindings.digest_mismatch`（退出码 1）**；缺失仍为现有 warning `ref.bindings.digest_missing`。已确定不一致按 error 处理，不保留“降级为 warning”的子选项。
   - 新增 `spec/ref/v1/declaration-digest.md` 冻结上述对象、规范化与 golden 向量；该文档不参与 `vendor/pact-ref-spec` 快照同步（`scripts/sync-spec.sh` 只同步 crate），本轮无 `pact-ref` 代码改动。
2. **第二步（运行期，已实现）**
   - 配置 `[ref_digest] mode = "warn" | "enforce" | "off"`，**默认 `warn`**；整段可省，`deny_unknown_fields` 拒绝未知键与未知模式。
   - 比较点：`POST /domains`（`load_domain`）授权后、**进入引擎前**，与该域当前加载绑定的 `metadata.declarationDigest` 比对；绑定未记录或该域无绑定 → 不比较；声明解析失败 → 保留既有 `Syntax`/`WellFormedness`。
   - `warn`：写一条 stderr 警告（`expected`/`actual`/`digestVersion`），`load_domain` 照常成功；`enforce`：仅“摘要存在且不一致” → `400 DeclarationDigestMismatch`（`detail` 含 `domain`/`expected`/`actual`/`digestVersion`），拒绝时域未装载、无事件写入。
   - 启动恢复不比较（既有实例持声明快照，D27）；显式重载（D48）不比对已登记声明——摘要约束的是“声明注册时对应哪版绑定”，既有实例不受换绑影响。
   - **版本**：字段保持 `sha256:<64hex>`，v1 语义固定在上面的规范文档；不新增版本字段、不改 schema。未来规范化变更必须引入新规范版本与新字段/新编号，禁止在同一字段静默换义；因此运行期只需实现 v1，不存在“算法版本不受支持”的比对分支。

**收益**：摘要只随语义变化，格式重排不误报；工具与 daemon 共用同一实现，golden 向量可锁一致性；lint 在提交前拦住真实漂移，运行期默认 `warn` 不会让误算直接阻断域加载。**代价**：需冻结规范化与条件文法并维护 golden；`gates` 条件参与意味着“只改条件”也要更新摘要（这是有意的：条件改变准入行为）。

**确认结果（2026-10-31）**：① 摘要对象按上表（语义接口，含 `gates` 条件与域名）；② 启用方式按“工具 + lint 先行（不一致 = error）、运行期后置且默认 `warn`、`enforce` 显式开启”；③ 摘要字符串保持 `sha256:<64hex>`（v1 固定在规范文档）。已登记 `FR34` / `D49`；两步均已实现，`[ref_digest]` 与 `DeclarationDigestMismatch` 已登记于 `docs/architecture.md` 附录 A。

### 13.5 ContractCard 生命周期：重启、显式重载与版本化（2026-10-29 定案；B0/B1 已实现）

**问题**：§13.4.1 第 4 条的“首版只在启动时加载、卡片变更需重启”是只约束首版，还是卡片的终身形态？域声明已经有一套靠迁移延续的生命周期（D27 实例创建时快照声明、D39 显式交接与活动实例别名）；卡片能否取得类似效果？

**先厘清两者为什么不能照搬**：

| 维度 | 域声明（D27 / D39） | ContractCard |
|---|---|---|
| 进入事件日志 / 重放 | 是：`DomainLoaded` / `Imported` 携带声明原文 | 否：引擎不读卡，只在 REST 边界使用 |
| 承载状态 | 是：规则、类型、gates 决定状态迁移 | 否：只描述“下游可依赖什么”与加严要求 |
| 变更的既有语义 | 新实例快照新声明，旧实例不变；换版须显式交接状态 | 原地改卡会追溯改变已写入 token 的归档资格 |
| 版本标识 | 实例键 + 基线事件 | **已内建**：卡是 `metadata.id@version`，绑定 `contract` 必须写成 `id@version` |

结论：卡片不需要“交接状态”，因为卡片不含状态；它需要的是**同版本不可变 + 版本化发布 + 原子激活**。绑定已经按 `id@version` 精确引用，这使“多版本共存”几乎零成本——缺的只是运行期加载（§13.4.1）与一个显式激活入口。

**真正的不变量：同一 `id@version` 的卡片内容不可变。** 无论选择启动加载还是热重载，“编辑同一版本的文件”都会让同一部署在不同时刻对同一 token 给出不同判定，而且不留版本痕迹；重启只是把这种漂移推迟到下次启动。因此这一条不应依赖 §13.5 的选择：lint 已用 `ref.contract.duplicate` 拒绝重复 `id@version`，运行期加载也必须同样 fail-closed。这也意味着“卡片变更”的正确定义是**发新版本**，不是改旧文件。

| 选项 | 机制 | 收益 | 代价 |
|---|---|---|---|
| **A. 仅启动加载**（§13.4.1 首版） | 重启即激活；无额外接口 | 实现最小；激活是原子、可审计的部署事件；无并发/半加载问题 | 卡错一个字也要重启 daemon，多域长跑部署代价高；无法按域灰度；仍要求同版本不可变，否则重启会静默改变行为 |
| **B. 版本化卡 + 显式重载**（推荐，分阶段） | 卡按 `id@version` 不可变、多版本共存；`POST /ref/reload`（或 CLI）把**卡片与绑定**作为一体，暂存解析校验后原子换索引；不做文件监听 | 与域“新版本服务新工作、旧版本继续可解析”效果一致；无需状态交接；旧绑定不变则旧 token 判定不变；可审计（`by` / `ts` / 每文件摘要） | 新端点与并发/回滚语义；卡片与绑定必须同批校验，避免引用悬空；旧版本需保留，目录清理要有规则；改绑定版本仍会改变历史 token 的归档判定（见下） |
| **C. 文件监听自动热加载** | inotify/轮询自动重载 | 零操作 | 读到半写文件、跨文件依赖不一致、与绑定配置竞态、不同实例观测到不同版本；难以审计与复现。**不推荐** |
| **D. 绑定与卡版本按实例冻结** | 实例创建时快照绑定/卡版本，之后不随 daemon 配置变化 | 与 D27 声明快照完全同构；历史 token 判定永久稳定 | 绑定目前是 daemon 级配置，需迁入实例控制面或事件基线；跨实例策略分裂；运维与 lint 复杂度显著上升。仅在有“按 token 追责到卡版本”的硬需求时再做 |

**推荐 B，分三阶段**：

1. **B0（`FR32` / `D47`，已实现 2026-10-30）**：启动加载；按 `contract_id()` 建索引；重复 `id@version`（无论内容是否相同）→ 启动失败；绑定声明的每张卡必须可解析。此时“变更卡片 = 发新版本 + 重启”，旧版本文件保留，旧绑定仍可解析。
2. **B1（`FR33` / `D48`，已实现 2026-10-30）**：新增显式重载入口，把 `[ref_contracts]` 与 `[ref_bindings]` 作为**同一事务**：解析并校验全部卡片与绑定 → 校验所有 `contract` 引用可解析 → 原子替换内存索引与规则；任一步失败保持旧状态并返回结构化错误，不做部分生效。审计记录操作者、时刻、每文件 `sha256`；`dryRun` 预检报告换绑后会有多少存量未钉版 token 无法归档。不做文件监听、不做 SIGHUP 隐式重载。实现为 `POST /ref/reload` 与 `pact ref reload [--dry-run] [--json]`；索引以 `RwLock<Arc<VisibilityProfiles>>` 原子换快照，在途请求持旧快照；审计写 `<data_dir>/ref-reload.jsonl`（append+fsync），ACL 要求覆盖当前与新索引中的全部绑定域（`{domain}/*`）。测试：`ref_reload_e2e` 7 项 + `pact-http` 单测 3 项 + CLI e2e 1 项。
3. **B2（按需，未登记编号）**：只有出现“必须证明某 token 产生时适用哪版卡”的审计需求时，才评估选项 D；那会改动基线/控制面，必须单独登记编号。

**关于回溯性加严**：D46 已定“归档钉版在 consume 时按当前有效规则判定”，所以把绑定切到更严的新卡版本，存量的未钉版 token 会变得不可归档——这是策略选择，不是重载机制的缺陷。缓解方式是把“预检报告 + 发布说明”作为 B1 的验收项；若要“只影响切换后产生的 token”，就必须记录 token 产生时的规则版本（即选项 D 的一类），不在 B1 范围内。

**已确认（2026-10-29）**：

1. “仅启动加载”是**首版激活机制**，不是卡片终身形态；
2. 采纳 **B0 的同版本不可变**（运行期重复 `id@version` 即启动失败），编号 `FR32` / `D47`，随 §13.4.1 一起实现；
3. **B1 显式重载**已单独立项并登记编号 `FR33` / `D48`，排在 B0 之后实现（2026-10-30 已实现）；
4. 接受**不做文件监听自动热加载**；
5. B2（按实例冻结）暂不立项，出现“按 token 追责到卡版本”的审计需求时再登记。

**依赖关系**：B0 与 §13.4.1 强绑定；B1 独立，但依赖 B0 的不可变性与 `contract_id()` 索引；B1 的重载校验必须与 `pact ref lint` 共用 `pact-ref-spec`，避免双实现漂移。域声明的 D27/D39 机制不变——卡片版本切换不触发交接，也不需要交接。

### 13.6 `refd` 的 resolve/describe 端点与鉴权（2026-10-31 定案）

本节解阻 §13.1 第 1 条的“`refd` 的 resolve/describe 端点”。规范产物是 `spec/ref/v1/reference-plane.openapi.yaml`；本节记录取舍与不变量。`invoke` 的只读契约见 §13.7；`revoke` 与 `mcp-mutating` 不在首版范围，随动态写阶段（§13.1 第 5 步）另行定案。

#### 13.6.1 不变量

1. **`refd` 不调用 `pactd`，`pactd` 也不调用 `refd`**（§8.6）。`refd` 看不到 PACT 的 read gate；参与者必须同时满足 PACT 的读权限与制品 ACL。
2. **`actor` 不来自请求体。** 主体由凭据验证后得出；请求体出现 `actor` 视为未知字段直接拒绝，避免自报身份与审计伪造。
3. **票面只用于选择 profile 与 adapter。** `domain` / `type` 是不透明选择器，只做长度与控制字符校验，不解释 PACT 语义。
4. **v1 只解析 URI；Identifier 是 `400 InvalidReference`。** 解析 Identifier 需要注册表，属后续可选阶段；拒绝而不是猜测。
5. **响应不含制品正文、内部端点或凭据。** `handle` 对参与者是不透明、短时的 bearer 值；服务端日志与审计不记录其明文。
6. **默认拒绝。** 未配置认证器时服务拒绝一切业务请求；ACL 未命中即拒绝；审计不可用时请求失败（fail-closed，可用显式配置放宽）。

#### 13.6.2 传输与版本

| 项 | 约定 |
|---|---|
| 路径 | 业务端点 `POST /v1/resolve`、`POST /v1/describe`、`POST /v1/invoke`；探针 `/healthz`、`/readyz` 不带版本 |
| 版本 | 破坏性变更升 `/v2`；v1 内只允许新增可选响应字段，客户端必须忽略未知响应字段，服务端拒绝未知请求字段 |
| 编码 | 仅 `application/json`（UTF-8）；其他 `415` |
| 大小 / 时限 | 请求体 ≤ 64 KiB（`413`）；单请求 30s 上限，adapter 超时映射 `504` |
| 缓存 | 业务响应 `Cache-Control: no-store`；`handle` 默认 300s 过期，且不得晚于 adapter 允许值 |
| 相关 ID | 可选请求头 `X-Trace-Id`（`[A-Za-z0-9._:-]{1,128}`）；缺省由服务生成，响应头回显，错误体同时携带 `traceId` |

#### 13.6.3 身份与凭据

认证模式由 `refd.toml` 的 `[auth] mode` 选择，**默认 `deny`**：

| 模式 | 用途 | 规则 |
|---|---|---|
| `oidc` | 生产默认 | JWT bearer；校验签名（JWKS，仅 HTTPS）、`iss`、`aud`（默认 `pact-ref`）、`exp`/`nbf`（偏差 ≤ 60s）；算法白名单 `RS256`/`ES256`/`EdDSA`，拒绝 `none` 与算法混淆；`sub` 作为主体；JWKS 有界缓存，遇未知 `kid` 刷新一次 |
| `static` | 仅引导/受控内网 | `[auth.static]` 必须显式 `allow = true`，否则启动失败；密钥常量时间比较；启动打印告警；变更需重启 |
| `deny` | 缺省 | 全部业务请求 `401`；服务仍启动以便探针与配置排错 |

- **不接受 PACT API key 作为 `refd` 凭据**：两者是不同受众。需要联合身份时用 OIDC / token exchange，不在请求体或 `ThingRef` 里传凭据。
- TLS 必须启用；只有绑定回环地址且显式 `allow_plaintext = true` 时才允许明文。
- 凭据解析失败一律 `401`，响应带 `WWW-Authenticate: Bearer realm="pact-ref"`，不区分“过期/签名错/未知主体”。

#### 13.6.4 授权

两层，顺序固定：

1. **端点 scope**：`/v1/resolve` 需要 `ref:resolve`，`/v1/describe` 需要 `ref:describe`，`/v1/invoke` 需要 `ref:invoke`；缺失 → `403 Forbidden`。
2. **对象 ACL**：`[[acl]]` 规则为**并集允许**、默认拒绝，无 deny 规则、无优先级排序：

```toml
[[acl]]
principal = "team-a/*"          # 主体 glob
domains   = ["build"]           # 或 ["*"]
types     = ["artifact"]
schemes   = ["https", "oci"]
operations = ["resolve:read", "resolve:verify", "describe", "invoke"]
```

- 任一规则匹配 `(principal, domain, type, scheme, operation)` 即允许；无匹配 → `403`，消息不区分“无权”与“不存在”。
- `ContractCard` 的 `parties` / `consumerScope` 是**声明**，不是执行点；执行点只有部署侧 ACL。
- `intent=verify` 与 `intent=read` 都归 `ref:resolve` scope，但在 ACL 的 operation 上可分别授权；`verify` 不返回 `handle`。
- ACL 判定在调用 adapter **之前**，避免无权调用者探测上游存在性。

#### 13.6.5 `POST /v1/resolve`

请求（全部字段均为必需，除非标注可选）：

```json
{
  "domain": "build",
  "type": "artifact",
  "reference": "https://artifacts.example.com/a.tar.zst",
  "intent": "read",
  "expectedDigest": "sha256:…",        // 可选
  "pactTokenId": "tok_…"               // 可选，仅用于审计关联，不作为授权依据
}
```

| 字段 | 规则 |
|---|---|
| `domain` / `type` | 1–255 字符，非控制字符；不解释语义 |
| `reference` | 非空 URI 字符串，≤ 65536；`∅`/`null`/Identifier/裸路径 → `400 InvalidReference` |
| `intent` | `read` 或 `verify`；必需，避免默认值掩盖调用方意图 |
| `expectedDigest` | 可选 `sha256:<64 hex>`；与实际不一致 → `409 DigestMismatch`（`detail` 带 expected/actual） |
| `pactTokenId` | 可选，≤ 128，`[A-Za-z0-9._:-]`；只写审计，永不用于授权 |

成功响应 `200`：

```json
{
  "contractId": "demo/artifact",
  "version": "1",
  "mediaType": "application/zstd",
  "digest": "sha256:…",          // 可选：adapter 未提供时省略
  "expiresAt": "2026-10-31T12:05:00Z", // read 时必需
  "handle": "https://…"          // read 时必需；verify 时禁止出现
}
```

- `handle` 由 adapter 提供，`refd` 只透传；v1 不提供 `/fetch` 代理，参与者自行兑换。
- `verify` 只做存在性与摘要校验，不返回 `handle`，也不下载正文（adapter 可选择 HEAD/Range 实现）。
- 解析结果不写入任何 PACT 状态；证据由参与者正常 `seed` / `produce` 产生。

#### 13.6.6 `POST /v1/describe`

请求：`{domain, type, reference}`（`domain` / `type` 必需，规则同上；`reference` 规则同上）。

响应是**消费者投影白名单**，不是整张 `ContractCard`：

| 包含 | 不含（永不自动透出） |
|---|---|
| `contractId`、`version`、`status`、`mediaType` | `metadata.owner`、`metadata.labels`、`metadata.description` |
| `artifact{kind, mediaType, schema?, schemaDigest?, digestPolicy}` | `parties{producer, custodian, consumers}` |
| `capability{kind, sideEffect, tool{name, version}, timeoutMs?}` | `capability.server`、`capability.auth.credentialSource` |
| `auth{audience, scopes}` | `resolution.endpoint`（内部拓扑） |
| `reference{schemes, pinning?, uriPattern?, mutableAllowed?}` | 任何未在 whitelist 中列出的新字段 |
| `lifecycle{retention, deletionPolicy, tombstoneRef?}` | |
| `failure`、`verification`、`consumerScope` | |

- 新增卡片字段不会自动出现在 `describe`；必须显式改本表与 OpenAPI，避免“加字段即泄露”。
- `describe` 只要求 `ref:describe`，不做 resolve 的 `intent` 区分；仍受对象 ACL 约束。
- 投影不含正文与摘要值以外的内容；`tombstoneRef` 是 URI，参与者可据此做删除可区分性检查。

#### 13.6.7 错误模型

统一信封：

```json
{ "code": "DigestMismatch", "message": "…", "traceId": "…", "retryable": false,
  "detail": { "expected": "sha256:…", "actual": "sha256:…" } }
```

| HTTP | `code` | 触发 | `retryable` |
|---|---|---|---|
| 400 | `InvalidRequest` | 未知字段、`intent` 非法、选择器超长 | false |
| 400 | `InvalidReference` | 空、`∅`、Identifier、裸路径、超长、profile 违规 | false |
| 401 | `Unauthorized` | 无凭据 / 凭据无效 | false |
| 403 | `Forbidden` | 缺 scope 或 ACL 未命中 | false |
| 404 | `NotFound` | adapter/registry 明确不存在 | false |
| 405 | `MethodNotAllowed` | 非 POST | false |
| 409 | `DigestMismatch` | `expectedDigest` 与内容不一致 | false |
| 410 | `Expired` | `handle` 兑换时已过期 | false |
| 413 | `RequestTooLarge` | 请求体超限 | false |
| 415 | `UnsupportedMediaType` | 非 JSON | false |
| 429 | `RateLimited` | 预留（v1 不实现限额，预留 `Retry-After`） | true |
| 500 | `Internal` | 未分类内部错误 | false |
| 500 | `AuditUnavailable` | 审计不可用且未放宽 | true |
| 502 | `UpstreamFailure` | adapter 上游失败 | true |
| 503 | `Unavailable` | 启动/排空中 | true |
| 504 | `UpstreamTimeout` | adapter 超时 | true |

- 401 与 403 严格区分；403 不透露目标是否存在。
- 错误消息不含正文、内部 URL、凭据或 `handle`。

#### 13.6.8 审计与可观测性

- 每个已认证请求一条事件，沿用 `ref-core::AuditEvent`：`traceId`、`actor`、`reference`、`domain`、`type`、`operation`、`contractId`、`status`、`resultDigest`、`pactTokenId`、`timestamp`。
- **不记录**：`handle` 明文、Authorization 头、请求体、内部 endpoint、上游查询串。
- 默认 fail-closed（审计写失败 → `500 AuditUnavailable`）；`[audit] fail_open = true` 需显式配置并在启动时告警。
- `/healthz` 只表示进程存活；`/readyz` 检查配置已加载与审计 sink 可用，失败 `503`，不返回内部细节。`oidc` 模式下 JWKS 可达性不在就绪探针中探测——JWKS 拉取失败在请求期以 `401` 体现，避免探针引入外部依赖与请求放大。

#### 13.6.9 实现映射与未决

| 位置 | 需要的改动 |
|---|---|
| `spec/ref/v1/reference-plane.openapi.yaml` | 已按本节重写为 v1 契约（resolve/describe + 探针）；只读 `invoke` 按 §13.7 增补 |
| `ref-core` | `ResolveRequest.actor` 移出请求、由服务端注入；`Resolution.handle` 改为 `Option`；`ContractDescriptor` 扩为投影结构 |
| `ref-auth` | 新增 OIDC 验证器、静态令牌验证器、ACL 求值器；`DenyAllAuthorizer` 作为缺省 |
| `ref-audit` | 落地 append-only sink 与 fail-closed 包装；现有 `MemoryAuditSink` 保留给测试 |
| `refd` | 引入 HTTP 栈（建议 `axum` + `tokio`，须单独做依赖与安全审视）；按 13.6.4 的顺序装配 |
| adapters | v1 至少实现 `file` 与 `https`；其余 adapter 按 §13.1 顺序推进 |

上述改动已于 2026-10-31 在 `pact-ref` 落地，含 `ref-auth` 的 OIDC 验证器（`[auth] mode = oidc`，见 FR35/D51）：
JWT/JWKS 验签、声明校验、`scope` 映射与统一 `401`；JWKS 文件/HTTPS 来源由 `refd` 注入，网络 IO 不进认证库。

未决（不阻塞首版实现）：

- `invoke` 的只读契约已按 §13.7 定案并实现（FR36/D52）；`revoke` 与 `mcp-mutating` 契约随动态写阶段定案；
- 可选注册表落地后，`describe` 可接受 Identifier，`resolve` 仍要求 URI；
- 限流、mTLS、多租户 JWKS 与认证配置热加载均不在 v1。

#### 13.6.10 明确不做

- `refd` 不读取 PACT 存储、不校验 PACT read gate、不替参与者做补偿。
- 不返回制品正文；不把 `handle` 当长期地址；不保证 `handle` 可跨调用复用。
- 不把 `ContractCard` 全文透出；`describe` 是白名单投影。
- 不对 Identifier 做模糊解析；不做隐式重试或降级。
- 不在 `ThingRef`、请求体或日志中存放凭据。

### 13.7 `POST /v1/invoke`：动态只读调用（2026-10-31 定案并实现）

`invoke` 是 §13.1 阶段 4 的落点。目标：把 `mcp-read` 能力调用结果**物化为新的静态引用**，让参与者回到 PACT 侧以正常 `seed`/`produce` 产生 witness token；`refd` 仍不写 PACT 状态。

#### 13.7.1 不变量

1. **只读**：仅接受 `capability.sideEffect = read-only` 的卡片；`mutating` 一律 `403`，不进入执行路径。
2. **引用归属**：`reference` 必须是绝对 URI，且以 `capability.server.id + "/"` 开头、工具路径非空；不满足 `400 InvalidReference`。
3. **服务登记**：`capability.server.id` 必须在部署侧 `[[mcp_servers]]` 登记；未登记不放行（启动期与请求期双重约束）。
4. **不写 PACT**：不产生 token、不改 Counter/Archive、不校验 PACT read gate；证据由参与者正常 `seed`/`produce` 产生。
5. **失败统一**：上游失败 → `502 UpstreamFailure`，不泄露上游细节；不隐式重试、不轮询。

#### 13.7.2 请求与响应

```json
{
  "domain": "query",
  "type": "answer",
  "reference": "mcp://tools.example/query.run",
  "operation": "query.run",
  "idempotencyKey": "req-1",
  "input": { "q": "…" }
}
```

| 字段 | 规则 |
|---|---|
| `domain` / `type` | 1–255 字符，非控制字符；不解释语义 |
| `reference` | 非空 URI，≤ 65536；必须以 `capability.server.id + "/"` 开头 |
| `operation` | 1–255 字符，非控制字符；能力侧操作名，不参与端点 scope 与 ACL 求值 |
| `idempotencyKey` | 必需，1–128，`[A-Za-z0-9._:-]`；v1 只透传上游，不本地去重 |
| `input` | 可选对象，缺省按空对象；原样传给 MCP `tools/call` |

成功响应 `200`：

```json
{ "status": "succeeded",
  "resultRef": "file:///var/lib/refd/materialized/<sha256>.json",
  "resultDigest": "sha256:<64hex>" }
```

- `resultRef` 是内容寻址的 `file://` 引用，可直接交给 `/v1/resolve`（`intent=read`/`verify`）。
- `resultDigest` 与 `resultRef` 同源；重复调用相同结果会收敛到同一路径。
- v1 的 `status` 只有 `succeeded`；失败走错误信封，不返回部分结果。

#### 13.7.3 授权与执行顺序

固定顺序，先授权后触达上游：

1. 认证（§13.6.3）→ 端点 scope `ref:invoke`；
2. 对象 ACL operation `invoke`（§13.6.4，`(principal, domain, type, scheme)` 匹配）；
3. profile 存在 → scheme 在卡片白名单 → `capability` 存在 → `sideEffect = read-only`；
4. 引用归属校验（§13.7.1 第 2 条）；
5. 调用 `InvokerRegistry` → `mcp` adapter（`spawn_blocking`，总上限 30s）；
6. 物化 → 审计 → 响应。

#### 13.7.4 物化与配置

- 结果按内容寻址写入 `[materialize].dir/<sha256>.json`：先写 `.tmp` 再 `rename`，避免半文件；已存在同摘要文件直接复用。
- `[materialize].max_bytes` 缺省 1048576，必须 > 0 且 ≤ 16 MiB；超限返回失败，不截断。
- `[[mcp_servers]]`：`id`（`mcp://…`）、`endpoint`（默认 `https://`，拒绝 userinfo/fragment）、`timeout_ms`（1–30000，缺省 10000）、`bearer_token_env`（必需且启动时非空）。
- `[[mcp_servers]].allow_plaintext`：仅当端点主机为回环（`localhost`/`127.0.0.0/8`/`::1`）时允许明文 `http://`；缺省 false，用于本地开发与测试。
- 启动 fail-closed：存在只读能力 profile 时，必须有 `[materialize]` 且 `capability.server.id` 已登记；`bearer_token_env` 指向的环境变量缺失或为空即拒绝启动。
- `capability.timeoutMs` 在 v1 不参与执行；实际超时取 `[[mcp_servers]].timeout_ms` 与 `refd` 30s 总上限的较小者。

#### 13.7.5 错误与审计

| HTTP | `code` | 触发 |
|---|---|---|
| 400 | `InvalidRequest` | 未知字段、选择器非法、`idempotencyKey` 非法 |
| 400 | `InvalidReference` | 引用非 URI、scheme 不在卡片白名单、不属于声明服务 |
| 403 | `Forbidden` | 缺 `ref:invoke`、ACL 未命中、`sideEffect` 非只读 |
| 404 | `NotFound` | profile 不存在 |
| 502 | `UpstreamFailure` | 上游明确失败（审计 `status=failed`） |
| 504 | `UpstreamTimeout` | 上游或总时限超时 |

- 审计沿用 §13.6.8 字段，`operation = "invoke"`；成功记 `status=ok` 与 `resultDigest`，上游失败记 `status=failed`。
- 不记录 `input`、上游响应正文、`resultRef` 路径以外的物化内容与任何 bearer 令牌。

#### 13.7.6 明确不做

- 不实现 `mcp-mutating`、`revoke`、审计边车与 witness/compensation（阶段 5）。
- 不做本地幂等表、结果缓存、轮询或自动重试。
- 不接受 Identifier 作为 `reference`；不做跨服务路由或服务发现。
- 不把上游响应直接透传给调用方；调用方只拿到物化引用与摘要。

### 13.8 `POST /v1/mutate`：动态写调用与幂等台账（2026-10-31 定案，实现待做）

`mutate` 是 §13.1 阶段 5 的落点（FR37/D53）。目标：为 `mcp-mutating` 能力提供与只读调用**完全分离**的写入通道，使写操作「至多生效一次、结果可追溯、崩溃不猜测」；`refd` 仍不写 PACT 状态。

#### 13.8.1 不变量

1. **端点与授权分离**：写只走 `/v1/mutate`，端点 scope `ref:mutate`，对象 ACL operation `mutate`。`/v1/invoke` 与 `/v1/mutate` 互不接受对方的 `sideEffect`；只读授权不自动升级为写授权。
2. **写意声明**：仅接受 `capability.sideEffect = mutating`；`read-only` 一律 `403`。
3. **严格归属**：`reference` 必须**严格等于** `<capability.server.id>/<capability.tool.name>`，`operation` 必须等于 `capability.tool.name`；比只读的前缀匹配更严，写不允许任意工具路径。不满足 `400 InvalidReference`。
4. **台账先于上游**：任何写 attempt 必须先落 `intent` 并 `fsync`，再调用上游；台账不可用即拒绝（`500 JournalUnavailable`），绝不「无台账写上游」。
5. **不写 PACT**：不产生 token、不改 Counter/Archive、不校验 PACT read gate；witness 由参与者正常 `seed`/`produce` 产生。
6. **不猜测**：崩溃窗口内的写标为 `in-doubt` 并阻断同键重试，宁可人工核对也不自动重试。

#### 13.8.2 `mcp-mutating` profile 必填字段

缺一即 `403 Forbidden`，且不触达上游：

| 字段 | 要求 |
|---|---|
| `capability.kind` | 非空 |
| `capability.sideEffect` | `mutating` |
| `capability.server.id` | 已登记于 `[[mcp_servers]]` |
| `capability.tool.name` | 非空，且 `reference`/`operation` 与之一致 |
| `capability.tool.inputSchemaDigest` | `sha256:<64hex>`，写输入契约必须可钉版 |
| `capability.idempotency.key` / `semantics` | 均非空；声明上游幂等键名与语义 |
| `capability.result.materialization` | `required` 或 `optional`；写结果必须声明是否物化 |
| `capability.auth.audience` | 非空；写操作必须声明目标受众 |
| `capability.failure` | 非空；写失败必须可解释，可含保留键 `compensation` |

**校验分层（2026-10-31 实现约定）**：上表中的**卡片自身字段**由 `pact-ref-spec` 的 `ContractCard::validate` 在加载/重载时强制（缺失即拒绝加载）；`server.id` 是否登记于 `[[mcp_servers]]`、`reference` 严格等于 `<server.id>/<tool.name>`、`operation == tool.name` 以及 `idempotencyKey` 文法属于**请求级**约束，由 `refd` 的 `/v1/mutate` handler 在触达上游前执行。

#### 13.8.3 请求与响应

```json
{
  "domain": "order",
  "type": "shipment",
  "reference": "mcp://erp.example/ship.create",
  "operation": "ship.create",
  "idempotencyKey": "order-42-ship-1",
  "input": { "orderId": "42" },
  "pactTokenId": "tok-..."
}
```

| 字段 | 规则 |
|---|---|
| `domain` / `type` | 1–255 字符，非控制字符；不解释语义 |
| `reference` | 非空 URI，≤ 65536；严格等于 `<server.id>/<tool.name>` |
| `operation` | 1–255 字符，非控制字符；必须等于 `capability.tool.name` |
| `idempotencyKey` | 必需，1–128，`[A-Za-z0-9._:-]`；**本地台账去重**，不只是透传 |
| `input` | 可选对象，缺省按空对象；参与 `requestDigest`，原样传给 MCP `tools/call` |
| `pactTokenId` | 可选，仅用于审计关联，永不用于授权 |

成功响应 `200`：

```json
{ "status": "succeeded",
  "resultRef": "file:///var/lib/refd/materialized/<sha256>.json",
  "resultDigest": "sha256:<64hex>",
  "replayed": false }
```

- `replayed=true` 表示该结论来自台账重放（上游未被再次调用）。
- `materialization = required` 时物化失败视为该次写失败；`optional` 时允许无 `resultRef`（仅 `status`）。

#### 13.8.4 授权与执行顺序

固定顺序，先授权、后上游；台账先于上游：

1. 解析 JSON（拒绝未知字段）→ 选择器与 `idempotencyKey` 校验 → 引用必须为绝对 URI；
2. 认证（§13.6.3）→ 端点 scope `ref:mutate`；
3. 对象 ACL operation `mutate`（§13.6.4）；
4. profile 存在 → scheme 在卡片白名单 → `capability` 存在且 `sideEffect = mutating`；
5. `mcp-mutating` profile 必填字段齐全（§13.8.2）→ 严格归属校验（§13.8.1 第 3 条）；
6. 构造 `requestDigest` → 台账查询（冲突/在途直接应答）；
7. 写 `intent` 并 `fsync` → 审计 intent（失败即中止，不触达上游）；
8. 调用 `InvokerRegistry` → `mcp` adapter（`spawn_blocking`，单次上限 30s）；
9. 物化（按 `materialization`）→ 写 `result` 并 `fsync` → 审计 result → 应答。

#### 13.8.5 幂等台账与崩溃窗口

- 文件：`[mutate].journal`（append-only JSONL，单写者，`O_APPEND` + 每次 `fsync`）。
- 记录：`intent { key_scope, request_digest, ts }` 与 `result { key_scope, request_digest, status, result_ref?, result_digest?, ts }`。
- 键作用域：`(actor, domain, type, reference, operation, idempotencyKey)`。
- 请求摘要：`sha256` over 紧凑 JSON（键按字典序）`{domain,type,reference,operation,input}`。
- 判定：

| 台账状态 | 行为 |
|---|---|
| 无记录 | 正常执行 |
| `intent` 无 `result` | `409 IdempotencyInDoubt`（不调上游、不重试） |
| `result` 且摘要相同、`succeeded` | 重放 `resultRef`/`resultDigest`，`replayed=true` |
| `result` 且摘要相同、`failed` | 重放失败结论 `502 UpstreamFailure`，`replayed=true` |
| 有记录但摘要不同 | `409 IdempotencyConflict` |
| 条目数达 `[mutate].max_entries` | `503 JournalFull`（不自动裁剪） |

- 崩溃窗口（显式接受，不自动修复）：

| 崩溃点 | 重启后状态 | 行为 |
|---|---|---|
| `intent` fsync 之前 | 无记录 | 上游未被调用，同键可重新执行 |
| `intent` 之后、上游返回前 | `intent` 无 `result` | `409 IdempotencyInDoubt` |
| 上游已执行、`result` fsync 之前 | `intent` 无 `result` | 同上（可能已生效，不猜测、不重试） |
| `result` fsync 之后 | 终态 | 重放该结论 |

- 启动：逐行解析，尾部半行（崩溃残留）截断并告警；完整非法行 ⇒ 拒绝启动。

#### 13.8.6 两阶段审计

- 沿用 §13.6.8 字段，`operation = "mutate"`；每次写 attempt 两条：`intent`（`status=ok`，仅记键摘要与 `requestDigest`）与 `result`（`status=ok|failed`，成功记 `resultDigest`）。
- `intent` 审计写失败 ⇒ 拒绝请求且**不触达上游**；`result` 审计写失败 ⇒ `500 AuditUnavailable`，但台账已落 `result`，重放仍可得确定结论。
- 不记录 `input`、上游响应正文与任何 bearer 令牌。

#### 13.8.7 witness 与 compensation

- 成功写后前端以 `resultRef` 调 `/v1/resolve` 取句柄，再由参与者在 PACT 侧 `seed`/`produce` 产出 witness token；`refd` 不写 PACT。
- `failure.compensation` 只是保留键，**不做自动 saga / 自动补偿**：补偿是另一张 `mutating` 卡片加域内规则显式调用，承担「补偿失败」的语义由域负责。
- 不提供撤销（`revoke`）；撤销若需要，是另一张写卡片。

#### 13.8.8 配置

```toml
[mutate]
journal = "/var/lib/refd/mutate-journal.jsonl"  # 存在 mutating profile 时必需
max_entries = 10000                            # 1..=1000000
```

- 复用 `[materialize]` 与 `[[mcp_servers]]`（§13.7.4）。
- 启动 fail-closed：存在 `mcp-mutating` profile 时，必须配置 `[mutate].journal`（父目录存在且可写）、`[materialize]`，且 `capability.server.id` 已登记；台账不可读/非法行或 `max_entries` 越界 ⇒ 拒绝启动。

#### 13.8.9 错误模型

| HTTP | `code` | 触发 |
|---|---|---|
| 400 | `InvalidRequest` | 未知字段、选择器非法、`idempotencyKey` 非法 |
| 400 | `InvalidReference` | 引用非 URI、scheme 越界、归属或 `operation` 不符 |
| 403 | `Forbidden` | 缺 `ref:mutate`、ACL 未命中、`sideEffect` 非 mutating、profile 必填字段缺失 |
| 404 | `NotFound` | profile 不存在 |
| 409 | `IdempotencyConflict` | 同键异摘要 |
| 409 | `IdempotencyInDoubt` | 同键上次结果不确定 |
| 500 | `JournalUnavailable` | 台账不可打开/追加/fsync |
| 500 | `AuditUnavailable` | 审计写失败（intent 阶段则不触达上游） |
| 502 | `UpstreamFailure` | 上游明确失败（审计 `status=failed`） |
| 503 | `JournalFull` | 台账条目达到上限 |
| 504 | `UpstreamTimeout` | 上游或总时限超时 |

#### 13.8.10 明确不做

- 不自动重试、不轮询上游、不做自动 saga/补偿、不提供 `revoke`。
- 不复用 `/v1/invoke` 承载写；不把 `ref:invoke` 视为写授权。
- 不做跨服务路由或服务发现。
- 不把上游响应直接透传给调用方；调用方只拿到物化引用与摘要。
- 台账不进入 PACT 事件日志，也不参与重放/压缩。

---

## 14. 引用位置

| 位置 | 引用方式 |
|---|---|
| `paper/pactv1_rev3.md` §1.2、§1.3、§1.4、§3.2、§4.4 | 规范源：token 结构、ThingRef 不透明、Rule-P、URI/Identifier/∅ |
| `docs/expressiveness-boundaries.md` | B11、B12、B20、X4；以及 B15/X3 原子多输出 |
| `docs/task-as-artifact.md` | 物引指向任务、任务状态/位置进类型 |
| `docs/architecture.md` 附录 A | 文档标识符与编号约定 |
| `crates/pact-core/src/types.rs` | `ThingRef(Option<String>)` 的当前形状 |
| `crates/pact-http/src/server.rs` | Counter/Archive 视图的域级授权与 `thing_ref` 暴露 |
| `crates/pact-wasm/src/host.rs` | WASM 门面：无 IO、宿主负责持久化 |
| `docs/wasm-local-spike-feasibility.md` | WASM 阶段、宿主边界与风险 |
| `spec/ref/v1/`（拟新增） | URI profile、RefBindings、ContractCard、Reference Plane OpenAPI |

## 15. 维护规则

1. 母本对 `ThingRef` 文法、Rule-P、`via` 或 CPN 映射作出修订时，同步本文件。
2. 本文件不替代母本；出现冲突以母本为准，并登记到 `docs/design-change.md`。
3. 新增 scheme、ContractCard 字段或 Reference Plane API 时，先更新 `spec/ref/v1/`，再更新本文件与实现。
4. 任何会影响准入的契约差异，不得只写进 ContractCard；必须先判断是否能编码成 `TypeName` 或 witness token。
5. 治理平面的网络/凭据/审计依赖不得进入 `pact-core`、`pact-engine`、`pact-store`。
6. 代码/文档引用本文件时写 `docs/thingref-governance.md §x`。
