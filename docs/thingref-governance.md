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
| `pact-analysis` | 零改动 | 可选：新增外部 lint，不进核心分析 |
| `pact-cpn` | 零改动 | ThingRef 仍是颜色集里的字符串 |
| `pact-store-model` | 可不变 | 字段形状不变；若要彻底不兼容才 bump 版本 |
| `pact-engine-kernel` | 零改动 | 只透传字符串 |
| `pact-engine` | 零改动 | 不解析、不 fetch、不校验存在性 |
| `pact-http` | 入口校验 + 视图可见性 + 视图审计 | `/seeds`、`/tokens` 校验；`/counter`、`/archive` 已实现默认 `read-gate` 过滤，RefBindings 的 `domain`/`hidden` 覆盖（FR28/D42）；视图级读审计已实现（`read_audit` 记录 `kind=view`，D43） |
| `pact-mcp` | 入口校验 | 与 REST 同一套校验；不新增治理语义 |
| `pact-cli` | 校验 + 离线 lint | `--thing-ref` 校验；`pact ref lint` 已实现（FR27/D41），离线读 RefBindings/ContractCard/`.pact` |
| `pact-wasm` | 纯校验 | URI 解析/绑定匹配可进 WASM；解析器/调用器不进 |
| `pact-runtime` | 参与者侧治理 | resolve / invoke / 审计边车最适合放这里 |
| `web/authoring` | 提示与校验 | 不涉及引擎 |

结论：**真正的耦合点在入口层和参与者运行时，不在引擎。**

### 8.2 单 WASM 是否仍能支撑

| 能力 | 单 WASM 能否支撑 | 原因 |
|---|---|---|
| 协调内核（token、规则、事件、快照） | 能 | 现有 `pact-wasm` 已经做到；无 IO |
| 静态治理（URI 校验、RefBindings 匹配、ContractCard 校验） | 能 | 纯函数；只要宿主把清单/卡片传进来 |
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
下一步是 `refd` 的 resolve/describe 端点。

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
   18 项单测与 wasm32 构建通过；`pact-ref` 已切换消费（当前 44 项测试通过）；`pact ref lint` 已实现（FR27/D41），
   覆盖绑定/契约/声明一致性与退出码。
   **待做**：`refd` 的 resolve/describe 端点。
2. **可见性阶段（已完成，2026-10-27）**：Counter/Archive 返回前过滤 `thing_ref`（FR28/D42）；
   默认 `read-gate`（无条件 `reads`/`consumes`），RefBindings 的 `domain`/`hidden` 显式覆盖；
   daemon `[ref_bindings].paths` 加载 profile，失败 fail-closed；令牌/签名模式下视图要求身份。
   **已完成**：视图级读审计（D43）——`read_audit = true` 时同一审计流记录 `kind=view`，按视图/类型聚合 `tokens/refs_visible/refs_filtered`，不记票面值；写失败 fail-closed。
3. **保留阶段（已完成，2026-10-27）**：ContractCard `lifecycle.retention` / `deletionPolicy` / `tombstoneRef`
   已在规范层收紧校验（FR29/D44）；新增 `Tombstone` 记录 schema 与 `pact ref lint --tombstones` 交叉检查。
   **已完成**：可发现性 lint——对 `required=true` 检查 hidden、视图读取路径、Rule-R/consume 路径、保留证据与删除可区分性（5 个 warning code，`--strict` 视为失败）。
4. **动态只读阶段**：`mcp-read` profile；resolve 适配器；物化结果。
5. **动态写阶段**：`mcp-mutating` profile；审计边车；witness/compensation。
6. **可选注册表**：只做发现，不做真相源。

### 13.2 测试

- **单元测试**：URI/Identifier 解析、RefBindings 匹配优先级、ContractCard 校验、retention/tombstone 校验、`∅`/缺失区分。
- **集成测试**：入口校验、Counter/Archive 可见性过滤、`thing_ref` 不进入 gate、witness token 证据链。
- **端到端测试**：静态制品 resolve → witness token → 下游生产；动态 `mcp-read` → 物化 → 新 token；tombstone 交叉检查。
- **回归测试**：旧 Identifier 仍可写入/读取；`ThingRef` 不透明性不变；CPN 颜色集不变。
- **安全测试**：SSRF、路径穿越、凭据泄露、日志注入、超长 URI、危险 scheme、URI userinfo。
- **WASM 测试**：静态校验可用；无网络依赖；AOT/体积门禁不退化。
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
