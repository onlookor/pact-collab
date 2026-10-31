---
name: pact-thingref-governance
description: >
  当用户要配置或审查 PACT 物引治理（ThingRef）时使用，包括 RefBindings 绑定规则、
  ContractCard 契约卡、Tombstone、票面可见性、declarationDigest 与显式重载。
  提供适用闸门、六个主题的配方、产物清单，并以 pact ref lint 与 pact ref digest 闭环验收。
  不用于运行域、部署 daemon、实现引擎或治理平面。
version: 0.1.0
tags: [pact, thingref, governance, ref-bindings, contract-card]
---

# PACT 物引治理

你是**治理配置的引导者与校验员**，不是代笔。目标：把“某个域的外部制品引用该怎么管”
落成一组可被 `pact ref lint` 验收的配置文件，并说清运行期会发生什么、不会发生什么。

## 0. 启用条件与边界

**启用**：给域配置引用规则、写契约卡、写 tombstone、决定票面可见性、对齐声明摘要、
修改绑定/卡片后如何生效。

**不启用，改走别的路径**：

| 请求 | 去向 |
|------|------|
| 运行域（seed / produce / read / consume / enabled / status） | `docs/manual.md` §4 |
| 启动 daemon、令牌、TLS、MCP、ACL | `docs/manual.md` §2 |
| 写或修改 `.pact` 域声明 | `pact-domain-authoring` skill |
| 实现 `refd` 的 resolve/describe、动态读写阶段、制品注册表 | 不在本 skill；属治理平面仓库与后续阶段 |
| 真正删除制品、撤销引用、发布 tombstone、抓取/调用被引用内容 | 不承诺；见 §7 |
| 只要一份“看起来像”的示例、不看真实域 | 拒绝生成；先问域与场景 |

## 1. 第一条动作：读取权威文档

本 skill 是执行器，**字段名、默认值、错误码以仓库文档与 schema 为准**，不得凭记忆填写。

按以下顺序定位，找到即停止：

1. 环境变量 `PACT_THINGREF_DOCS` 指向的目录；
2. 当前仓库（含 `Cargo.toml` 与 `crates/pact-*`）：
   `docs/manual.md` §7.8–§7.14 与 §9、`spec/ref/v1/`、`docs/thingref-governance.md` §8 与 §13；
3. 都没有 → **停下来问用户要路径**；不要即兴发明字段。

按需读取，不必一次全读：

| 文件 | 用途 |
|------|------|
| `spec/ref/v1/ref-bindings.schema.json` | 绑定规则的全部字段与取值 |
| `spec/ref/v1/uri-profile.md` | 什么样的值算 URI / Identifier / ∅、钉版形态 |
| `spec/ref/v1/contract-card.schema.json` | 契约卡字段、`digestPolicy`、`lifecycle` |
| `spec/ref/v1/tombstone.schema.json` | tombstone 必填项与时间格式 |
| `spec/ref/v1/declaration-digest.md` | 摘要计算对象与 golden 向量 |

## 2. 不变量（违反任何一条即视为失败）

1. **引擎不解析票面**。校验只在接口层：REST 写入边界，或 WASM 宿主的写入前校验。
   不要承诺引擎会 fetch、invoke、解引用、撤销或删除制品。
2. **卡片只能加严**。卡片可以要求归档钉版；不能放宽绑定已声明的策略。
3. **改声明语义就要重算摘要**。类型、前置、gates、seed 任一变化都会改变
   `declarationDigest`；不改绑定就会 lint error，`enforce` 模式下域加载被拒。
4. **`required: true` 不能配 `value: none`**。这是配置矛盾，daemon 直接拒绝启动。
5. **重载是一次性全局替换**。没有文件监听、没有自动回滚、没有按域灰度；发布前必须先 `--dry-run`。
6. **`visibility` 不是授权**。它只决定视图是否返回票面，不改变 token 存在性、类型、计数，
   也不改变 `GET /tokens/{id}` 的 Rule-R 授权。

## 3. 六个主题

### 3.1 绑定 RefBindings

**何时用**：需要规定某域“哪些类型必须有引用、允许哪些 scheme、是否必须钉版、
谁能在视图里看到票面”。

**三态**（写入时）：

| 发送方 | 含义 | 结果 |
|--------|------|------|
| 不带 `thing_ref` 字段 | 未表态 | `400 ThingRefMissing` |
| `"thing_ref": null` | 确认没有外部制品（`∅`） | 成功；`required: true` 时 `400 ThingRefRequired` |
| `"thing_ref": "…"` | 有引用 | 校验后写入规范化值（scheme 小写） |

CLI/MCP 省略参数时发送显式 `null`（= `∅`），不会产生 `ThingRefMissing`；WASM 宿主同理。

**最小骨架**：

```yaml
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj                      # ^[A-Za-z0-9_-]{1,128}$
  declarationDigest: sha256:…       # 可选；见 3.5
spec:
  defaults:
    ref:
      visibility: read-gate         # read-gate / domain / hidden
  types:
    design:
      ref:
        required: true
        schemes: ["https", "oci"]
        pinning: required-at-archive
        maxLength: 4096
        visibility: domain
```

**规则字段**：`required`（默认 false）、`allowIdentifier`（默认 false）、`schemes`
（`*` 或 scheme 文法）、`maxLength`（1–65536）、`value`（`none`/`uri`/`identifier`）、
`pinning`（`required`/`required-at-archive`/`optional`）、`onUnknownScheme`
（`reject` 缺省 / `allow` / `treat-as-identifier`）、`contract`（`id@version`）、
`visibility`。

**继承顺序**：`defaults → scheme → type → instance/type`。写规则前先确认改哪一层，
不要在每个类型里重复默认值。

**必须问的四个问题**：这个类型是否允许“没有外部制品”？是否还接受旧的裸 Identifier？
允许哪些 scheme？归档前是否必须钉版？

**验收**：`pact ref lint --bindings <文件或目录> --declaration <file.pact>`，
error 清零；运行期把同一份文件配进 `[ref_bindings].paths`。

### 3.2 契约卡 ContractCard

**何时用**：需要跨域复用同一套制品策略，或需要声明保留期与删除策略。

- 卡片以 `metadata.id@version` 精确引用；**同一 `id@version` 出现两次即拒绝启动**，
  内容变更必须提升 `version`。
- 绑定用 `contract: id@version` 引用卡片；引用了却未配置 `[ref_contracts].paths`
  或卡片不可解析 → daemon 拒绝启动。
- 只能加严：`reference.pinning.requiredForArchive: true` 或
  `artifact.digestPolicy: required-at-archive` 会让 consume 也要求钉版；
  写入（seed/produce）不受卡片钉版影响。

```yaml
apiVersion: pact.ref/v1
kind: ContractCard
metadata: { id: se/design-review, version: 2 }
spec:
  artifact:
    digestPolicy: required-at-archive   # required-at-archive / optional / never
  lifecycle:
    retention: P365D                    # permanent 或正 ISO 8601 duration
    deletionPolicy: tombstone-then-delete
    tombstoneRef: urn:pact:tombstone:se/design-review
```

`tombstone-then-delete` 必须给 URI 形式的 `tombstoneRef`；其余策略不得声明。
归档钉版契约不得用 `deletionPolicy: delete`。

**验收**：`pact ref lint --bindings … --contracts <文件或目录> --declaration …`。

### 3.3 Tombstone

**何时用**：制品已删除且需要留下“它曾经存在、为何消失”的证据。

```yaml
apiVersion: pact.ref/v1
kind: Tombstone
metadata:
  id: urn:pact:tombstone:se/design-review:req-001   # 全局唯一 URI
spec:
  contract: se/design-review@2
  ref: https://artifacts.example.com/design/req-001
  reason: expired                                   # expired/replaced/revoked/compromised/legal/manual
  deletedAt: 2026-10-27T00:00:00Z                   # RFC3339 UTC，大写 Z
  retainUntil: 2027-10-27T00:00:00Z                 # 可选，必须晚于 deletedAt
  replacedBy: https://artifacts.example.com/design/req-002   # reason=replaced 必填
  note: 保留期结束，按契约删除                        # ≤2048 字节
```

- **PACT 不发布 tombstone**：tombstone 是普通制品，由域作者通过正常 `seed`/`produce` 写入，
  票面指向它。
- `metadata.id` 全局唯一；同一 `(contract, ref)` 多条记录会告警。
- 被引用契约必须存在，且删除策略必须是 `tombstone-then-delete`；`ref`/`replacedBy`
  的 scheme 与长度必须在契约允许范围内。

**验收**：`pact ref lint --bindings … --contracts … --tombstones <文件或目录>`。

### 3.4 票面可见性

**何时用**：票面含敏感位置（内部 URL、私有仓库），需要控制哪些调用者能在视图里看到。

| profile | 行为 |
|---------|------|
| `read-gate`（缺省） | 调用者对该类型有无条件 `reads`/`consumes` 才返回；`produces` 不授予 |
| `domain` | 通过视图认证的调用者都能看到 |
| `hidden` | 任何视图都不返回 |

- 配置来源：`[ref_bindings].paths`；不配即全部 `read-gate`。
- 只影响 `GET /counter` 与 `GET /archive`；token 存在性、类型、状态、计数不变。
- `GET /tokens/{id}` 仍由 Rule-R 的 read gate 决定。
- 启用 `read_audit` 后，视图读取按类型记录暴露计数（`kind=view`，不含票面值）；
  审计写失败 → 视图 `500`。

**验收**：`pact ref lint --bindings … --declaration …` 的
`ref.discoverability.*` 告警（`required_hidden`、`view_read_path_missing`、
`no_reader`、`retention_missing`、`deletion_without_tombstone`）逐条裁决。

### 3.5 declarationDigest

**何时用**：绑定需要绑定到某一版声明，防止“声明改了、绑定没跟上”。

摘要对象是声明的**语义接口**：域名 + 类型 + 前置条件 + gates（含条件表达式）+ 种子授权；
注释、空白、段内顺序、文件名不参与。v1 语义冻结。

```bash
pact ref digest se_lifecycle.pact                 # 打印 sha256:<64hex>
pact ref digest se_lifecycle.pact --name proj     # 显式域名；缺省取文件 stem
pact --json ref digest se_lifecycle.pact          # 含 algorithm / digestVersion
```

把输出写入 `metadata.declarationDigest`，然后：

- `pact ref lint --declaration …`：未记录 → warning `ref.bindings.digest_missing`
  （`--strict` 失败）；已记录但不一致 → error `ref.bindings.digest_mismatch`（退出 1）。
- 运行期 `[ref_digest].mode`：`off` 不比、`warn`（缺省）只警告、`enforce` 返回
  `400 DeclarationDigestMismatch` 且不加载域。
- **只在 `POST /domains`（含 CLI `domain load` / `domain handoff`）比较**；
  启动恢复与 `pact ref reload` 不比较。
- 只比“绑定里确实有摘要”的域；没有摘要不会被强制要求。
- 声明本身解析/良构失败仍报 `Syntax` / `WellFormedness`，不会改写成摘要错误。

**处置顺序**：声明有意变更 → 重新 `pact ref digest` → 更新绑定 → `pact ref reload` → 重试加载。

### 3.6 显式重载

**何时用**：改完绑定或卡片，不重启 daemon 就生效。

```bash
pact ref reload --dry-run    # 只校验并报告影响，不替换
pact ref reload              # 校验 → 审计 → 原子替换
```

REST 等价入口：`POST /ref/reload`，体 `{"dryRun": true|false}`，响应含
`applied`、`bindings`、`contracts`、`blockedTokens[]`、`blockedTotal`。

- **同事务**：卡片与绑定一起读取、校验、替换；任一步失败保留旧规则。
  在途请求持旧快照完成，新请求用新索引。
- **预检**：`blockedTokens` 统计换绑后因缺摘要而新增无法归档的存量 token，只给计数。
- **审计**：每次尝试向 `<data_dir>/ref-reload.jsonl` 追加一行（append+fsync）；
  审计写失败 → 拒绝本次重载。
- **认证/授权**：配置身份后必须认证；启用 ACL 时须对当前与新索引中每个绑定域有
  `{domain}/*`。
- **不做**：文件监听、SIGHUP、自动回滚、按域灰度。

**错误码**：`RefReloadInvalid`（400）、`RefReloadBusy`（409）、`Forbidden`（403）、
`Unauthorized`（401）、`RefReloadUnavailable` / `RefReloadFailed` /
`RefReloadAuditFailed`（500）。

## 4. 工作流

### 探测工具（只做一次）

```bash
pact --version
./target/release/pact --version
./target/debug/pact --version
```

- **找到** → 记下命令前缀，走校验闭环。
- **没找到** → 问用户三选一，不要擅自安装：给出二进制路径 / 同意
  `cargo build -p pact-cli` / 只做静态自检并在交付物顶部标注“未验证”。

### 采集

每轮最多问三个问题，每个给建议默认值并标“假设”。必须拿到：

| 槽位 | 问什么 | 落到哪 |
|------|--------|--------|
| 域与声明 | 域名、声明文件、是否已有绑定 | `metadata.domain`、`--declaration` |
| 外部制品 | 哪些类型真的有制品，哪些允许没有 | `types.<t>.ref.required` |
| 引用形态 | 只允许 URI 还是还兼容旧 Identifier | `allowIdentifier`、`schemes`、`value` |
| 钉版 | 写入时还是归档前必须带摘要 | `pinning`、卡片 `digestPolicy` |
| 可见性 | 票面是否敏感、谁能看 | `visibility` |
| 留存与删除 | 保留多久、删除是否需要 tombstone | 卡片 `lifecycle`、Tombstone |

### 起草

顺序：绑定 → 契约卡 → tombstone → 可见性 → 摘要。先给用户看草案（标注假设），
确认后再落盘。默认路径沿用项目约定（如 `refs/<domain>.refs.yaml`、`cards/`、`tombstones/`）；
项目没有约定时先问。

### 校验闭环

1. 非 strict：`pact ref lint --bindings … [--contracts …] [--tombstones …] [--declaration …]`，
   error 必须清零。
2. strict：加 `--strict`，warning 逐条“修掉”或写明保留理由。
3. 声明改动过 → `pact ref digest` 重算并回写绑定，再回到第 1 步。
4. 最多 5 轮；仍不过就停下来说明卡点，不要无限试错。

### 上线

```bash
pact daemon start                 # 绑定/卡片加载失败会拒绝启动
pact ref reload --dry-run         # 看 blockedTokens
pact ref reload
```

运行期验证：`GET /counter` / `GET /archive` 的票面是否符合预期；需要归档的类型
未钉版时 consume 应返回 `400 ThingRefUnpinned`。

## 5. 完成标准（Definition of Done）

全部满足才算完成：

1. `pact ref lint` 退出码 0；用户要求严格时 `--strict` 也退出 0。
2. 发现过的每条 warning 都有一句裁决理由（修掉，或说明为何保留）。
3. 绑定 `declarationDigest` 与声明一致（使用摘要时）。
4. `pact ref reload --dry-run` 的 `blockedTokens` 已向用户说明，并有发布决定。
5. 交付物包含：文件路径、lint 输出、运行期影响、明确的“不承诺”清单。

## 6. 输出模板

````markdown
## 治理配置结果

- 域：<domain>（声明 <file>，摘要 <sha256:…>）
- 关键裁决：<最多 3 条>

### 产物

| 文件 | 作用 |
|------|------|
| <path> | RefBindings |
| <path> | ContractCard / Tombstone（如有） |

### 验证

```console
$ pact ref lint --bindings … --contracts … --tombstones … --declaration …
<摘要行；exit 0>
```

### 运行期行为

- 写入：<三态与必填规则>
- 归档：<是否要求钉版>
- 可见性：<profile 与影响范围>
- 重载：<dry-run 的 blockedTokens；是否已发布>

### 边界

- 不 fetch / 不 invoke / 不撤销 / 不删除内容
- <WASM 或未生效项>

### 下一步

```bash
pact ref reload --dry-run && pact ref reload
```
````

## 7. 边界与不承诺

- `pact ref lint` 只做**静态**检查：不验证票面可达、不验证内容已删除、不访问网络、
  不读运行期状态。lint 通过不等于运行期引用可用。
- PACT 引擎与 daemon 不 fetch、不 dereference、不 invoke 被引用内容，不撤销引用，
  不发布 tombstone，不执行删除，不做保留计时。
- 契约卡的 `artifact.kind` / `mediaType` 检查未生效。
- WASM 本地宿主只做写入侧结构校验与绑定校验；卡片检查与归档钉版只在 REST 边界。
- 不实现 `refd` 的 resolve/describe、动态只读/写入阶段、可选注册表。
- 不提供制品迁移、镜像、重写历史的方案；这些是治理平面之外的运维动作。

## 8. 常见失败模式（自查）

- 只跑通 lint 就宣称“运行期可用” → 必须写清归档/可见性/重载影响。
- 改了声明不重算摘要 → `enforce` 下域直接被拒。
- 以为卡片能放宽绑定 → 卡片只能加严。
- 配 `required: true` + `value: none` → daemon 拒绝启动。
- 以为 `pact ref reload` 会自动回滚 → 它只做原子替换。
- 把 `visibility: domain` 当授权 → 它只控制视图是否返回票面。
- 给 WASM 宿主承诺卡片校验 → 不支持。
- `--json` 写在子命令之后 → 它是全局 flag，须在子命令前。
- 编造 schema 字段名或默认值 → 回 §1 查 schema。
- 一次抛出十几个问题 → 每轮 ≤3 个。

## 9. 引用

| 文件 | 用途 |
|------|------|
| `docs/manual.md` §7.8–§7.14 | 命令、配置、错误码、运行期行为的权威说明 |
| `docs/manual.md` §9 | REST 错误码总表 |
| `spec/ref/v1/uri-profile.md` | URI / Identifier / ∅ 的判定与钉版形态 |
| `spec/ref/v1/ref-bindings.schema.json` | 绑定字段与取值 |
| `spec/ref/v1/contract-card.schema.json` | 契约卡与 `lifecycle` |
| `spec/ref/v1/tombstone.schema.json` | tombstone 必填项 |
| `spec/ref/v1/declaration-digest.md` | 摘要对象、规范化与 golden 向量 |
| `docs/thingref-governance.md` §8 / §13 | 治理平面边界与阶段划分 |
