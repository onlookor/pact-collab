# PACT Declaration Digest v1

本文件是 `pact.ref/v1` 的 `declarationDigest` 规范，对应 RefBindings 的
`metadata.declarationDigest`。它只规定**摘要对象、规范化形式与摘要字符串**，
不规定绑定匹配、运行期加载策略或契约卡内容。

摘要用于回答一个问题：“这份 RefBindings 是为哪一版域声明生成的？”它让
“声明被改动但绑定忘记更新”变成可检测的漂移，而不是运行期的静默错配。

## 1. 摘要对象

摘要覆盖域声明的**语义接口**，即影响绑定匹配、票面可见性与准入判定的部分：

| 字段 | 参与 | 说明 |
|---|---|---|
| 域名 | 是 | `load_domain` 的 `name`；CLI 取 `--name`，缺省为声明文件 stem；须匹配 `[A-Za-z0-9_-]{1,128}` |
| `types` | 是 | 排序后的类型集合 |
| `preconditions` | 是 | 每个右部类型及其排序后的前驱集合 |
| `gates` | 是 | 每个 `(participant, action, type)` 与其条件表达式 |
| `seed_authority` | 是 | 每个 `(participant, type)` |

以下**不参与**摘要：

- 注释、空白、换行与缩进、Unicode 箭头与 ASCII `->` 的书写差异；
- 段内条目顺序、声明文件路径与文件名；
- 域实例名、静态分析结果；
- RefBindings、ContractCard、Tombstone 的内容。

同一正文在不同域名下必须得到不同摘要，因此域名一律参与。

## 2. 规范化形式

规范化文本为 **UTF-8 紧凑 JSON**：无缩进、字段间无空格、无尾换行。
顶层对象字段顺序固定，不得增删：

```json
{"kind":"pact.declaration.interface","v":1,"domain":"<name>","types":[],"preconditions":[],"gates":[],"seed":[]}
```

各字段的规范形式：

| 字段 | JSON 形态 | 排序 / 去重 |
|---|---|---|
| `kind` | 常量 `"pact.declaration.interface"` | — |
| `v` | 常量 `1` | — |
| `domain` | 字符串 | — |
| `types` | 字符串数组 | 按 Unicode 码点字典序（即字节序）升序，集合去重 |
| `preconditions` | `[{"type":<rhs>,"predecessors":[<lhs>…]}]` | 按 `type` 升序；`predecessors` 升序去重 |
| `gates` | `[{"participant":…,"action":…,"type":…,"condition":<null|string>}]` | 按 `(participant, action, type)` 字典序升序；无条件的 `condition` 为 `null` |
| `seed` | `[{"participant":…,"type":…}]` | 按 `(participant, type)` 字典序升序，集合去重 |

`action` 取 `"read"`、`"consume"`、`"produce"` 之一。排序键是**字符串字典序**，
不是枚举声明顺序；例如同一参与者下 `consume < produce < read`。
空集合输出 `[]`，空条件输出 `null`，两者都不省略字段。

摘要字符串为 `sha256:` 后接上述字节的小写十六进制 SHA-256（共 71 字符）：

```text
declarationDigest = "sha256:" 64*HEXDIG-LOWER
```

## 3. 条件表达式文本

`gates` 的条件按下列文法序列化为字符串（类型名文法不含逗号与括号，故无歧义）：

```text
expr    = "via(" type ")"
        / "exists(" type ")"
        / "count(" type "," op "," n ")"
        / "and(" expr "," expr ")"
        / "or(" expr "," expr ")"
        / "not(" expr ")"
op      = ">=" / "="
```

`and` / `or` 保留声明中的结合结构，不做重排、交换或化简；`not` 保留嵌套。
`n` 为十进制正整数（PosNat，无前导零）。条件文本只表达语法结构，不表达运行时真值。

## 4. 工具与漂移语义

离线工具：

```bash
pact ref digest se_lifecycle.pact                # 打印 sha256:<64hex>
pact ref digest se_lifecycle.pact --name proj    # 指定域名（缺省取文件 stem）
pact --json ref digest se_lifecycle.pact         # {"declarationDigest":…,"algorithm":"sha256","digestVersion":1}
```

`pact ref lint --declaration` 在能读到声明时按本规范计算摘要，并与绑定
`metadata.declarationDigest` 比对：

| 绑定状态 | 结论 | 严重性 |
|---|---|---|
| 未记录 `declarationDigest` | `ref.bindings.digest_missing` | warning（`--strict` 失败） |
| 已记录且与实际一致 | 无 finding | — |
| 已记录但不一致 | `ref.bindings.digest_mismatch` | error（退出码 1） |

不一致表示声明与绑定对不上，须用 `pact ref digest` 重新生成并更新绑定。
缺失仍只是 warning：未使用摘要的部署不受影响。

运行期比对（`POST /domains` 的 `load_domain`）**已实现**，由 `[ref_digest].mode`
控制严格程度（`off` 不比较 / `warn` 缺省，只警告 / `enforce` 返回
`400 DeclarationDigestMismatch` 且不加载域）。只在 `load_domain` 比较，启动恢复与
显式重载不比较；只比较绑定中确实记录了摘要的域。其边界见 `docs/design.md` §9.16（D49）
与 `docs/manual.md` §7.14。

## 5. Golden 向量

以下向量冻结规范化与哈希；实现与测试必须逐字节一致。

### 5.1 基础声明

声明（注释与空白可任意变化，语义相同）：

```pact
types { request; design; impl }
preconditions { design → impl }
gates {
  architect: reads design, consumes design, produces impl;
  dev: produces impl
}
seed_authority { client: seeds request }
```

域名 `se_lifecycle`，规范化 JSON：

```json
{"kind":"pact.declaration.interface","v":1,"domain":"se_lifecycle","types":["design","impl","request"],"preconditions":[{"type":"impl","predecessors":["design"]}],"gates":[{"participant":"architect","action":"consume","type":"design","condition":null},{"participant":"architect","action":"produce","type":"impl","condition":null},{"participant":"architect","action":"read","type":"design","condition":null},{"participant":"dev","action":"produce","type":"impl","condition":null}],"seed":[{"participant":"client","type":"request"}]}
```

摘要：

```text
sha256:feade2dd1b4e7b5e3fe4ffc1619decf1cae0c217e7d1c8b2629756a1569d76e3
```

### 5.2 最小声明

声明：

```pact
types { t }
preconditions { }
gates { p: reads t }
seed_authority { p: seeds t }
```

域名 `minimal`，规范化 JSON：

```json
{"kind":"pact.declaration.interface","v":1,"domain":"minimal","types":["t"],"preconditions":[],"gates":[{"participant":"p","action":"read","type":"t","condition":null}],"seed":[{"participant":"p","type":"t"}]}
```

摘要：

```text
sha256:320e4491c4bdade4d9956db27fc42127104699110028f4cac9aff0feeaf21300
```

### 5.3 条件表达式

声明：

```pact
types { a; b; c }
preconditions { a → b }
gates {
  p: produces b,
     reads b when via a and not (exists b or count c >= 2),
     consumes b when count a = 1
}
seed_authority { s: seeds a }
```

域名 `cond`，`gates` 中的条件文本为：

```text
and(via(a),not(or(exists(b),count(c,>=,2))))
count(a,=,1)
```

（摘要值不在此单列，由实现逐字节计算；测试校验条件文本片段。）

### 5.4 仓库示例锚点

以下为仓库内示例文件按文件名 stem 计算的摘要，用作跨版本回归锚点：

| 文件 | 域名 | `declarationDigest` |
|---|---|---|
| `examples/domains/se_lifecycle.pact` | `se_lifecycle` | `sha256:0316b1d92cafafef1ede488c48418afbd09e7d425440bde1e2db1e712cdd2078` |
| `examples/domains/re_pipeline.pact` | `re_pipeline` | `sha256:8750b59b7b981d0618aaee2b4efdd3dffef166d65c04d819770bd47ec8a17f87` |

示例文件内容或文件名变化都会改变锚点，须同步更新本表与对应回归测试。

## 6. 版本与兼容

- `v` 固定为 `1`，语义冻结在本文件。字段保持 `sha256:<64hex>`；不新增版本后缀，
  不改 `ref-bindings.schema.json` 的 `pattern`。
- 任何改变摘要对象、字段顺序、排序规则或条件文本的变化都属于**新规范版本**：
  必须引入新的 `v`（或新字段/新编号）并更新 golden 向量，禁止在同一字段静默换义。
- 因此运行期只需实现 v1 比对；不存在“算法版本不受支持”的降级分支。
- RefBindings 与 `.pact` 声明是摘要的两个输入；契约卡与 Tombstone 不参与，
  其生命周期按 `FR32`/`FR33` 独立管理。
