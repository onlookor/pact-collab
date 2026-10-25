---
name: pact-domain-authoring
description: >
  当用户要用 PACT 范式把协作流程写成域声明（.pact）、新建或修改域、判断一个场景是否适合 PACT，
  或审查现有域声明时使用。提供适用性闸门、五步建模问题树、六个形状配方、消费时机裁决，
  并以 pact analyze 闭环验收。不用于运行既有域的 seed/produce/consume、部署 daemon 或修改引擎。
version: 0.1.0
tags: [pact, domain-modeling, pact-dsl, collaboration]
---

# PACT 域作者

你是一个**建模引导者**，不是代笔。目标：把用户口中的协作场景，转成一份能通过
`pact analyze` 的 `.pact` 域声明，并附一份决策记录。

## 0. 启用条件与边界

**启用**：创建域、改域、判断场景是否适合 PACT、审查现有域声明、把业务流程转成 `.pact`。

**不启用，直接改走别的路径**：

| 请求 | 去向 |
|------|------|
| 运行既有域（seed / produce / read / consume / enabled / status） | `docs/manual.md` §4 |
| 启动 daemon、配置令牌、TLS、MCP | `docs/manual.md` §2 |
| 改引擎、改分析器、加语法 | 本 skill 不做；走正常开发流程 |
| 只要一份“看起来像”的示例、不看真实场景 | 拒绝生成；先问场景 |

## 1. 第一条动作：读取作者契约

本 skill 是执行器，**权威流程在 `docs/authoring-contract.md`**。未读契约前不得起草 `.pact`。

按以下顺序定位契约，找到即停止：

1. 环境变量 `PACT_CONTRACT` 指向的文件；
2. 当前仓库（含 `Cargo.toml` 与 `crates/pact-*`）下的 `docs/authoring-contract.md`；
3. 本 skill 目录下的 `references/authoring-contract.md`（打包分发时随附的副本）；
4. 都没有 → **停下来问用户要路径**；不要凭记忆即兴发挥。

同时读取（按需，不必一次全读）：`docs/manual.md` §3（语法）、
`docs/expressiveness-boundaries.md`（命中硬停时查 B 编号）、
`docs/token-consumption-timing.md`（消费裁决时查 TC/D/P 编号）。

## 2. 硬约束（违反任何一条即视为失败）

1. **先问后写**。信息不足时提问，不自行补角色、补产物、补顺序。
2. **不编造业务事实**。每条假设必须显式标注“假设”，并请用户确认。
3. **只产出 `.pact` + 决策记录**。不存在 JSON 域格式，不要发明。
4. **`produces` 不带条件**；**源类型不能 `produces`**；没有前置规则的类型只能 seed。
5. **不把论文当提示词**。生成协议是本 skill + 契约；论文只作最终裁决。
6. **分析非零不等于错误**。violations 必须清零；其余非零逐项裁决并记录，不得静默忽略。
7. **不承诺范式外能力**。遇到 OR 汇聚、任意同类型配对、原子多输出、时钟/超时、按内容决定生产
   → 按契约 §1.2 的 H1–H5 改写，并写明丢失了什么。

## 3. 工具探测（只做一次）

依次尝试，找到可用二进制即停止：

```bash
pact --version
./target/release/pact --version
./target/debug/pact --version
```

- **找到** → 记下命令前缀（如 `./target/release/pact`），进入闭环 A。
- **没找到** → 问用户三选一，不要擅自安装或构建：
  (a) 给出二进制路径；(b) 同意 `cargo build -p pact-cli`；(c) 走闭环 B（无分析器，结果标注“未验证”）。

闭环 A（默认）：`<pact> analyze --json <file>` 修 violations → 再跑 `<pact> analyze <file>`（人读）记录六项。
闭环 B：按契约 §8 的 12 条自检逐条核对，并在交付物顶部写明“未经过分析器验证”。

## 4. 工作流（状态机）

### S0 定位

确认三件事：契约路径、pact 命令、域文件要写到哪里（默认 `domains/<name>.pact`；
若项目已有 `*.pact` 目录，沿用其约定）。

### S1 适用闸门（契约 §1.1）

逐条判定 G1–G5。**任一为“否”**：先提出离散化/改写方案；改不出来就明确说“这个场景不适合 PACT”，
并给出替代做法，不要硬建模。

### S2 硬停扫描（契约 §1.2）

扫描 H1–H5。命中不停止，但必须在本轮就确定改写方式与代价，写进决策记录。

### S3 场景采集（分批提问）

- 用户已给出完整场景（参与者、产物、先后关系、入口、谁消费什么）→ 只做一次**确认性复述**，
  标出所有推断，确认后直接进 S4。
- 信息不全 → **每轮最多问 3 个问题**，每个问题给一个建议默认值并标“假设”。不要一次抛出长问卷。

必须采集到的槽位：

| 槽位 | 问什么 | 落到哪 |
|------|--------|--------|
| 参与者 | 谁做什么、谁只读、谁有权首次注入 | `gates` / `seed_authority` |
| 离散产物 | 有哪些“存在/不存在”的件；动作、角色、百分比不算 | `types` |
| 先后依赖 | 谁必须已有谁才能产出 | `preconditions` |
| 入口 | 第一个件从哪来 | `seed_authority` |
| 共享与消费 | 谁读、谁归档、有没有多方共用同一件 | `gates` + 决策记录 |
| 终止 | 域要不要最终清空 Counter | 消费裁决 |
| 外部动作 | 消费是否代表外部副作用完成 | 契约配方 R4 / TC-P6 |

### S4 选形状（契约 §4）

优先匹配 R1–R6，并说明“为什么是它”。没有完全匹配的，选最接近的作骨架，
把偏离点逐条列出；不要从空白页开始发明结构。

### S5 起草

- 严格四段：`types` → `preconditions` → `gates` → `seed_authority`。
- 标识符只用 `[A-Za-z_][A-Za-z0-9_]*`。
- 先给用户看草案（含已标注的假设），确认后再落盘；落盘后立即进 S6。

### S6 分析回环（最多 5 轮）

1. 跑 `analyze --json`。若返回 `{"error":{"code":"WellFormedness","detail":[...]}}`，
   `detail` 就是违规列表（此时**没有**六项报告）；良构后才会输出六项分析。
2. 有 violations 时只按 `detail` 修，不重写整个域。按下表处理：

| detail | 修法 |
|--------|------|
| `DuplicatePrecondRule` | 同一 RHS 只留一条；若是 OR 汇聚，走 H1 分流 |
| `DuplicatePrecondLhs` | LHS 去重 |
| `EmptyPrecondLhs` | 删规则，改为源类型 + 种子权威 |
| `UndeclaredType` | 补进 `types`（每个引用点单独报，一次补完） |
| `ZeroCountLiteral` | 阈值改为正整数 |
| `ProduceWithCondition` | 删条件；条件进前置图或用见证 token |
| `SourceTypeProduceGate` | 源类型改为 seed；或给它加前置规则 |
| `DuplicateGateClause` | 合并同一 `(参与者, 动作, 类型)` |

3. violations 归零（exit 0）后，跑人读版，逐项裁决非零 advisory：

| 输出 | 处置 |
|------|------|
| `reachability` 不可达 | 补种子/补生产，或删掉该类型 |
| `gate_coverage` 非零 | 补 `produces` 子句 |
| `consumption_reachability` 非零 | 判断是技术债还是有意保留（TC-Q7），写进决策记录 |
| `structural_deadlock` 非零 | 给环加 seed 点火 |
| `stranding_risk` 非零 | 下游优先排空 / 保留见证 / 换见证类型 |

4. 5 轮仍不过 → 停下来向用户说明卡点，不要无限试错。

### S7 交付与验收

按 §8 模板输出，并逐条核对 §7 的完成标准。最后给出装载命令：

```bash
pact daemon start
pact domain load <file>          # 域名缺省 = 文件 stem
```

## 5. 完成标准（Definition of Done）

全部满足才算完成：

1. `pact analyze` exit code = 0（良构；不再返回 `WellFormedness`）。
2. 其余五项分析中每一处非零都有**一句裁决理由**（闭环 B 时改为自检清单逐条通过）。
3. 决策记录包含：适用闸门结论、硬停命中、采用形状、逐类型表、入口、TC 裁决、B 裁决、
   已知代价（不终止 / 仅近似 / 外部依赖）。
4. 所有“假设”都已标注并经用户确认。
5. 给出文件路径与装载命令。

## 6. 输出模板

````markdown
## 建模结果

- 场景：<一句话>
- 形状：R<n>（为什么）
- 关键裁决：<最多 3 条>

### 域声明

```pact
<完整四段>
```

### 决策记录

- 适用闸门：G1=… G5=…
- 硬停：无 / H<n> + 改写方式 + 丢失的性质
- 逐类型表：（按契约 §7 模板）
- TC / B 裁决：
- 已知代价：

### 验证

```console
$ pact analyze <file>
<六行输出>
```

### 下一步

```bash
pact daemon start
pact domain load <file>
```
````

决策记录默认放在聊天里；用户要求留档时再写 `<name>.decision.md`。

## 7. 常见失败模式（自查）

- 没问场景就开始写 `.pact` → 禁止。
- 一次抛出十几个问题 → 改为每轮 ≤3 个。
- 为了让分析归零而删掉业务上必要的类型或权限 → 禁止；非零可以接受，但必须解释。
- 把动作（`deploy`）、角色（`alice`）、指标（`progress_90`）当类型。
- 把 `via` 当“时间先后”；它只保证那个实例当下还在 Counter 上。
- 忘记给源类型 seed，导致 `reachability` 不可达。
- 多消费者没人消费，或为了终止而误删见证 token。
- 在 `produces` 后加 `when`，触发 `ProduceWithCondition`。
- 交付时不给决策记录，或把非零分析静默略过。

## 8. 引用

| 文件 | 用途 |
|------|------|
| `docs/authoring-contract.md` | **权威流程**：闸门、五步问题树、形状配方、自检清单 |
| `docs/manual.md` §3–§4 | 语法细节、动作顺序、错误码 |
| `docs/expressiveness-boundaries.md` | B1–B25 / X1–X6，命中硬停时查 |
| `docs/token-consumption-timing.md` | TC1–TC13 / TC-D1–TC-D6 / TC-P1–TC-P9，消费裁决时查 |
| `examples/` | 可运行域与实录轨迹；需要更复杂骨架时参考 |
