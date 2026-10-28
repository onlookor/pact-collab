# PACT Reference URI Profile v1

本文件是 `pact.ref/v1` 的票面规范，对应母本 `ThingRef ::= URI | Identifier | ∅`。
它只规定**如何判定与校验一个引用字符串**，不规定制品内容、存储位置或解析协议。

## 1. 三种形态

| 形态 | 线上表示 | 含义 |
|---|---|---|
| URI | 带合法 scheme 的字符串，如 `https://example.com/a` | 新引用优先使用 |
| Identifier | 不带 scheme 的短标识，如 `REQ-001` | 兼容旧数据；标记为“未分类引用” |
| `∅` | JSON `null`；入口缺失则拒绝 | 明确“没有外部制品” |

**缺失 ≠ `∅`。** 请求体未提供字段是输入错误（HTTP 400）；显式 `null` 才是 `∅`。
token、事件与日志保留两种状态，不得把缺失折叠成 `∅`。

## 2. 词法规则

```text
reference    = uri / identifier / absent
absent       = null
uri          = scheme ":" rest
scheme       = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
identifier   = 1*( ALPHA / DIGIT / "_" / "-" / "." / ":" / "/" / "@" / "#" )
```

规范性约束：

| # | 规则 | 处理 |
|---|---|---|
| R1 | 不得为空或只有空白 | 拒绝 `EmptyReference` |
| R2 | 默认最大 2048 字节；ContractCard/RefBindings 可下调，硬上限 65536 | 拒绝 `TooLong` |
| R3 | 不得包含 ASCII 控制字符（U+0000–U+001F、U+007F） | 拒绝 `ControlCharacter` |
| R4 | 不得有首尾空白 | 拒绝 `MalformedUri` |
| R5 | 不得是裸路径：`/`、`./`、`../`、`~/`、`\`、Windows 盘符 | 拒绝 `BarePath` |
| R6 | URI authority 不得含 `userinfo`（`user:pass@`） | 拒绝 `UserInfoNotAllowed` |
| R7 | scheme 按小写规范化；绑定表中的 scheme 必须小写 | 规范化 / 绑定校验失败 |
| R8 | Identifier 不含 scheme，属于 legacy；新引用 SHOULD 使用 URI | lint 警告；是否允许由 RefBindings 决定 |
| R9 | ThingRef 不得承载凭据、令牌或秘密 | 安全拒绝；凭据走通道层 |

## 3. 合法与非法示例

合法：

```text
https://artifacts.example.com/se/design#sha256=9f2c...
git+ssh://git@example.com/org/repo@v1.2.3
oci://registry.example.com/team/image@sha256:...
file:///srv/artifacts/report.md
urn:pact:artifact:build/42
mcp://build.example.com/build-server/build.run@3
REQ-001
null
```

非法：

```text
(空字符串)                       -> EmptyReference
/tmp/report.md                  -> BarePath
C:\artifacts\report.md          -> BarePath
https://user:pass@example.com/a -> UserInfoNotAllowed
https://example.com/a\nb        -> ControlCharacter
 https://example.com/a          -> MalformedUri（首部空白）
```

## 4. 钉版

1. Archive 时若契约要求钉版，URI 必须带内容摘要（如 `#sha256=<hex>`），或在 `Resolution` 中返回
   `digest` 字段。
2. 摘要格式固定为 `sha256:<64 位小写十六进制>`；其他算法必须新增字段/版本，不得复用。
3. `mutableAllowed=false` 的契约不得接受可变 URL 作为最终 pin；可变引用只能作为定位线索，
   最终 token 必须升级为钉版引用或显式 `∅`。

## 5. 解析边界

| 项 | 规范 |
|---|---|
| 解析者 | 仅治理平面（`pact-ref`）；PACT 引擎与 Rule-P/GateExpr 不解析 ThingRef |
| 未知 scheme | 默认拒绝；RefBindings 可显式 `allow` 或 `treat-as-identifier` |
| 取回 | 由适配器完成；必须有超时、大小上限与 SSRF/路径穿越防护 |
| 失败 | 必须显式返回；不得把失败伪装成 `∅` |
| 动态能力 | `mcp://` 调用结果必须物化为新的 URI 或 `∅`；不得只返回内存值 |

## 6. 版本与兼容

- `apiVersion` 固定为 `pact.ref/v1`。
- 新增可选字段属于兼容变更；改变判定语义、收紧既有合法输入或改变 `∅` 含义必须升到 `v2`。
- Identifier 的兼容期不设自动截止；是否允许由 RefBindings 的 `allowIdentifier` 显式决定。
