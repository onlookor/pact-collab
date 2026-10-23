# PACT 用户手册

本手册面向 **PACT 范式系统的使用者**：要用它建模自己的协作流程、跑起来、看分析结果、做审计的人。
内容与任何具体技能、行业、方法论无关——需求工程、软件工程、agent 循环、跨工具编排都只是它的实例。

引用约定：

| 记号 | 指向 |
|------|------|
| `§x.y` | 范式规范 `paper/pactv1_rev3.md`（论文母本，本手册不复述其推导） |
| `设计 §x.y` | `docs/design.md`（实现决策与线上格式） |
| `架构 §x.y` | `docs/architecture.md`（进程与数据流视图） |
| `示例 §x` | `examples/README.md`（五份通用域 + 两条可复现轨迹） |
| `pact` | CLI 二进制，下文假定 `alias pact=./target/debug/pact` |

---

## 1. 这套系统给你什么

### 1.1 一句话

PACT 把你的协作流程写成一份 **域声明**（`.pact` 文件），然后由一个 **daemon** 持有唯一权威的
`Counter`（在制品集合），按六条规则裁决每一步动作：**谁**（Participant）**在什么前提下**
（Precondition）**能对什么**（Token 的 Type）**做什么**（Gate：read / consume / produce），
被消费的东西进 `Archive` 且不可逆。

它不替你干活，也不调度你。它只做三件事：**拒绝非法动作**、**记录每一步**、**告诉你现在能做什么**。

### 1.2 六原语与六规则

| 原语 | 是什么 | 在 `.pact` 里怎么写 |
|------|--------|---------------------|
| Token | 不可变的产出实例，有 ULID id、类型、前驱、生产者 | 不写——运行时产生 |
| Participant | 动作主体（人、agent、工具） | `gates` / `seed_authority` 段的主语 |
| Precondition | 类型级前置图，`Precond(T)` = 生产 T 所需的前驱类型集合 | `preconditions { A → B; {A + C} → D }` |
| Gate | 静态授权矩阵 `(参与者, 动作, 类型) ↦ 可选条件` | `gates { dev: reads design, consumes design, produces impl }` |
| Counter | 当前在场 token 集合 | 不写——运行时状态 |
| Archive | 已消费 token 的只增账本 | 不写——运行时状态 |

| 规则 | 效果 | 命令 | 关键约束 |
|------|------|------|----------|
| Rule-S 种子注入 | 无中生有放入一个 token | `pact seed` | 豁免前置条件，但必须 `(参与者, 类型) ∈ SeedAuthority` |
| Rule-P 生产 | 由前驱产出新 token | `pact produce` | **前驱必须仍在 Counter 上**，且类型集合恰等于 `Precond(T)`；前驱不被消费 |
| Rule-R 读取 | 取回 token 全量数据 | `pact read` | 非破坏；只对 Counter 上的 token 成立 |
| Rule-E 消费 | token 移出 Counter 进 Archive | `pact consume` | 显式、无级联、**不可逆** |
| Rule-W 自驱 | 问「我现在能生产什么」 | `pact enabled` / `pact_subscribe` | 引擎被动，不替你决定 |
| Rule-T 终止 | `Counter = ∅` 且无人可 Rule-P | `pact status` 的 `terminated` | 信息性判定，**不锁定域实例** |

三条最容易踩的性质：

1. **Rule-P 不消费前驱**。`design → implementation` 之后 `design` 仍在 Counter 上，
   可以被别人再读、再作为前驱。想让它消失必须显式 `consume`。
2. **Rule-E 无级联、不可逆**。没有撤销命令。消费顺序错了只能重载域重来。
3. **Rule-T 是信息，不是闸门**。域终止后不禁止动作，只是 Counter 空了让 Rule-P 拿不到前驱；
   再 `seed` 一次，`terminated` 就翻回 `false`（示例 §3 第 11 步实测）。

### 1.3 四个接入形态，一个权威

```
agent ── MCP (stdio) ─────────┐
agent ── MCP (Streamable HTTP)─┤
应用  ── REST + SSE ───────────┼── 同一个 daemon 进程 ── Counter 唯一权威 ── JSONL 事件日志
人    ── CLI ──────────────────┘   （pact-mcp / 内建 /mcp / pact-cli 在线命令都落到 REST）
```

四形态能力对等，共享同一套线上格式（设计 §6）。差别只在传输、身份与形态：
stdio 与 REST 可用静态 Bearer 或 Ed25519 签名；`/mcp` 可用 OIDC JWT 或静态 Bearer；
`/health` `/ready` `/metrics` 免身份。唯一语义例外是 `compose`，见 §6.3。

### 1.4 什么时候该用（§2.1 五条判据）

五条**全部**满足才划算：

| # | 判据 | 含义 | 反例信号 |
|---|------|------|----------|
| 1 | Discrete Artifacts | 流程产物是可数的离散件（文档、报告、构建、工单） | 产物是连续量（温度、进度百分比） |
| 2 | Precedence Relations | 「B 必须在 A 之后」是流程的**本质**约束而非偏好 | 各步骤本来就能任意乱序 |
| 3 | Transferable Outputs | 产出要在参与者之间传递 | 单人自娱自乐、无交接 |
| 4 | Participant Count ≥ 1 | 至少一个参与者；**单参与者是合法退化情形** | — |
| 5 | Identifiable Entry Points | 每个源类型都能指认一个种子权威 | 说不清「第一份东西谁给」 |

第 5 条常被误读为「必须有源类型」。不是的：`examples/domains/react_loop.pact` 的四个类型各有
唯一前驱（纯循环，§1.4 意义下没有源类型），第 5 条**空满足**，但仍然需要
`seed_authority { user: seeds observation }` 点火——**SeedAuthority 不只服务于源类型，
它也是给纯循环域提供启动推力的手段**（seed 豁免前置条件，§1.4.3）。

### 1.5 什么时候别用（§2.3 九条边界）

| 你的需求 | 为什么不适用 | 绕法 |
|----------|--------------|------|
| 连续过程（信号、流量、进度） | 范式是离散 token 的 | 离散化为里程碑 token |
| 实时约束（超时、截止、时延） | 无时钟原语，`Timestamp` 只是记录 | 超时逻辑放参与者侧，用 `thing_ref` 挂外部工单 |
| 概率分支（按 likelihood 走不同路） | 规则是确定性的 | 概率决策留在参与者内部，Counter 只承接其结果 |
| OR 分支（A **或** B 都能触发 C） | 前置图只支持合取（§1.4.4） | 拆成两个类型 `C_from_A` / `C_from_B`，或统一到一个前驱类型 |
| 产物的内部结构（章节、字段、AST） | Token 不可变、内部不透明 | 用 `ThingRef` 委派给外部系统 |
| 自动生命周期排序（自动推进到下一阶段） | 设计如此（§1.6，零隐式动作） | 由参与者显式 produce/consume；agent 用 Rule-W 自驱 |
| 生产时的额外 gate 条件 | 设计如此（§1.5.2，`produces` 子句不带条件） | 把约束表达进前置图（增加前驱类型） |
| 两次消费之间的时序（先消费 A 才能消费 B） | `GateExpr` 只评估当前状态（§1.7.3） | 把时序编码进类型与前置图 |
| 内部生产源类型 | 设计如此（§1.4，源类型只经 Rule-S 进入） | 给它指定 seed 权威，或改成有前驱的非源类型 |

「设计如此」的四行不是缺陷清单，是范式的边界声明：PACT 刻意不做隐式动作，
否则 Counter 就不再是唯一可信的协调介质。

---

## 2. 装起来、跑起来

### 2.1 构建

```bash
cargo build -p pact-cli        # → target/debug/pact（含 daemon start）
cargo build -p pact-http       # → target/debug/pactd（裸 daemon，等价于 pact daemon start）
cargo build -p pact-mcp        # → target/debug/pact-mcp（stdio MCP 薄封装，由 agent 拉起）
```

### 2.2 配置文件 `pact.toml`

```toml
data_dir    = "./data"            # 事件日志 / 快照 / 索引目录
listen_unix = "./pact.sock"       # 默认监听；单机安全
# listen_tcp  = "127.0.0.1:8787"  # 可选，与 listen_unix 可同时启用
# pid_file    = "./pactd.pid"     # 可选；启动写入、退出删除（含陈旧检测）
# compact_on_start = false        # 可选；启动恢复后自动压缩全部实例日志（D25）
# compact_keep_last = 0           # 可选；自动压缩保留的尾部事件条数
# read_audit = false              # 可选；持久化 Rule-R 读审计（每次成功读追加一行记录，默认关闭）

# NFR6 v2：TLS / mTLS（可选；仅对 listen_tcp 生效）
# [tls]
# certificate = "/etc/pact/server-chain.pem"   # PEM 证书链
# private_key = "/etc/pact/server.key"         # PEM 私钥（PKCS#8/PKCS#1/SEC1）
# client_ca   = "/etc/pact/client-ca.pem"      # 存在即要求客户端证书（mTLS）

# NFR6 v2：Ed25519 公钥表；非空即启用请求签名（值 = base64 的 32 字节公钥）
# [auth.keys]
# architect = "<base64-32B-pubkey>"

[mcp]
enabled = true
# public_url = "https://pact.example.com"      # OIDC 必填；也用于 /mcp Host 白名单

# NFR6 v2：MCP OIDC 资源服务器（仅 /mcp；不内置授权服务器）
# [mcp.oidc]
# issuer = "https://idp.example.com"
# audience = "pact-mcp"
# principal_claim = "sub"                      # 缺省 sub
# principal_map = { "auth0|alice" = "alice" }  # claim 值 → 参与者名
# jwks_uri = "https://idp.example.com/.well-known/jwks.json"  # discovery=false 时必填
# discovery = true                             # 缺省 true
# allowed_algs = ["RS256", "ES256", "EdDSA"]   # 缺省即这三项
# ca = "/etc/pact/idp-ca.pem"                  # IdP TLS 自定义 CA
# allow_static_tokens = true                   # 允许静态 Bearer 与 JWT 并存
# allow_insecure_http = false                  # 仅本地/测试可 true

[auth]                            # 可选；整段缺省 = 信任声明模式
[auth.tokens]
architect = "sekret"              # 参与者 → 令牌，请求头 Authorization: Bearer

# [auth.acl]                      # 可选；域级 ACL，非空时 [auth.tokens] 或 [auth.keys] 至少其一
# architect = ["proj/*", "other/default"]  # 条目格式：domain/instance 或 domain/*
```

缺省语义（文件不存在或字段缺省时）：`data_dir = "./data"`、`listen_unix = "./pact.sock"`、
无 TCP、无 TLS、无签名公钥表、无令牌表、无 ACL、无 PID 文件、不自动压缩（`compact_on_start = false`、`compact_keep_last = 0`）、
不写读审计（`read_audit = false`）、`[mcp].enabled = false`、无 OIDC。三个易踩点：

- 配置用 `deny_unknown_fields`，**写错键名会直接报错**，不会静默忽略。
- `listen_unix = ""`（空串）= 禁用 unix socket；只写 `listen_tcp` 而不写 `listen_unix`，
  unix socket 也一并禁用。
- `[tls]` 必须同时给 `certificate` 与 `private_key`，且必须配置 `listen_tcp`；只配其中一个会拒绝启动。

### 2.3 启动与停止 daemon

```bash
pact daemon start --config pact.toml      # 前台运行；Ctrl-C 或 SIGTERM 优雅停止
```

**没有 `daemon stop` 子命令。** 停止就是终止进程（`kill <pid>`）；
`examples/online/run.sh` 的 `cleanup()` 就是这么做的，并用 `trap ... EXIT` 保证兜底。
状态不会丢：JSONL 是真相源，下次启动自动做「快照 + 其后事件重放」恢复（架构 §2.3）。

**优雅停机（D25）**：SIGINT（Ctrl-C）与 SIGTERM 走同一路径——停止接受新连接 → 最多等 10 秒
让在途请求完成 → 写终态快照 → 删除 PID 文件与 unix 套接字。SSE 长连接会占满宽限窗口，
超时后强制进入快照并退出（不阻塞）。

**PID 文件（可选）**：配置 `pid_file` 后启动写入自身 PID、退出删除。启动时若该文件已存在
且记录的 PID 仍存活，则**拒绝启动**（防两个 pactd 争同一文件）；确认是无用残留后再手动删除。

### 2.3.1 运维端点（免身份）

daemon 提供三个仅供本地/内网使用的运维端点（**不校验身份**）。认证边界：配置 `[auth.acl]` 后，域生命周期、四类动作、查询、`/compact` 与读审计视图全部按 `(domain, instance)` 授权（§2.4、§7.6）；**未配置 ACL** 时不做域级授权（静态 Bearer 仍用于四类动作端点的参与者一致性；启用签名时验签覆盖全部非豁免 REST 端点，生命周期、查询与 `/compact` 也需签名）。`/health` `/ready` `/metrics` 与 OIDC 元数据始终免身份；`POST /validate` 在**未启用签名**时免身份，启用 `[auth.keys]` 后需签名。daemon 默认应只监听 unix socket 或回环；需要暴露 TCP 时启用 TLS/mTLS（设计 §9.5），**不得把明文端口暴露给不可信网络**。

```bash
curl --unix-socket ./pact.sock http://localhost/health   # {"status":"ok"}（存活）
curl --unix-socket ./pact.sock http://localhost/ready    # 就绪；引擎不可应答时 503
curl --unix-socket ./pact.sock http://localhost/metrics  # Prometheus 文本格式
```

`/metrics` 指标：`pact_domains_declared`、`pact_instances_loaded`、
`pact_counter_size{domain,instance}`、`pact_archive_size{domain,instance}`、
`pact_terminated{domain,instance}`（1/0）、`pact_uptime_seconds`。仅统计**已加载**的域实例。

### 2.4 身份与令牌

- **信任声明模式**（REST 默认，`[auth.tokens]` 与 `[auth.keys]` 均为空时）：任何 `--participant` 值都被接受。身份校验退化为「你说你是谁就是谁」，权限裁决仍由 Gate 与 SeedAuthority 完成。OIDC 配置只影响 `/mcp`，不会让 REST 变成需要 JWT。
- **令牌模式**：令牌表非空时，参与者必须在表内且 `Authorization: Bearer <token>` 匹配，否则 `401 Unauthorized`。比较用恒时算法（防 CWE-208 时序侧信道）。
- **签名模式（NFR6 v2）**：`[auth.keys]` 非空时启用 Ed25519 请求签名，客户端用 `--sign-key`（base64 32 字节种子）+ `--sign-as` 对每个 REST 请求签名；服务端验签后把参与者写入可信头。启用签名后，除豁免端点外的 REST 请求**必须签名，Bearer 不能替代**；两者同时配置时，带签名的请求以签名身份为准。请求中的 `participant` 必须等于签名身份，否则 `401`。
- **MCP OIDC（NFR6 v2）**：只作用于 `/mcp`，daemon 是资源服务器；客户端把 IdP 签发的访问令牌直接作为 `Authorization: Bearer` 传入，客户端不做授权码流程。OIDC 令牌不用于 REST，也不会被转发。

身份在**命令进入引擎之前**由接口层校验；核心与引擎不感知认证方式（设计 §9.4/§9.6/§9.8，NFR5）。

**域级 ACL（可选）**：`[auth.acl]` 非空时启用实例级授权（`domain/instance` 或 `domain/*`），覆盖生命周期、动作、查询、压缩与读审计视图；未配置时保持上述信任声明/令牌/签名行为不变。配置必须同时提供 `[auth.tokens]` 或 `[auth.keys]`（至少其一），否则 daemon 拒绝启动。详见 §7.6。

### 2.5 分发与部署（阶段 9，D26）

仓库 `dist/` 提供三条等价部署路径，载荷是同一批静态二进制：`pact`（CLI + daemon）、
`pactd`（裸 daemon）、`pact-mcp`（stdio MCP 薄封装）、`pact-runtime`（生产编排）。

#### 2.5.1 musl 静态二进制打包

前置：

```bash
rustup target add x86_64-unknown-linux-musl
# C 依赖（turso）需要 musl C 工具链：
#   Fedora/Aurora: dnf install musl-gcc
#   Debian/Ubuntu: apt install musl-tools
#   Alpine:        apk add musl-dev gcc
```

```bash
dist/build-static.sh
# → dist/target/pact-0.1.0-x86_64-unknown-linux-musl.tar.gz
#   dist/target/pact-0.1.0-x86_64-unknown-linux-musl.tar.gz.sha256
```

脚本行为：`cargo build --release --locked --target x86_64-unknown-linux-musl --bins`；
四个二进制逐个 strip 并用 `readelf -d` 核验**无 `NEEDED` 动态依赖**（失败即中止，不回退 glibc）；
归档内含 `bin/`、`pact.toml.example`、`lib/systemd/system/pactd.service`、`LICENSE`、`SHA256SUMS`。
缺前置时脚本显式报错并给出安装提示。`TARGET`、`MUSL_CC`、`NO_STRIP` 可覆盖缺省。

#### 2.5.2 systemd

```bash
# 1) 系统用户与目录（StateDirectory/RuntimeDirectory 由 systemd 创建并归 pact 所有）
useradd --system --home-dir /var/lib/pact --shell /usr/sbin/nologin pact

# 2) 安装二进制与 unit
install -m 0755 bin/pact bin/pactd bin/pact-mcp bin/pact-runtime /usr/local/bin/
install -m 0644 lib/systemd/system/pactd.service /etc/systemd/system/pactd.service

# 3) 配置（含令牌时收紧权限）
install -d -m 0755 /etc/pact
install -o root -g pact -m 0640 pact.toml.example /etc/pact/pact.toml

# 4) 启动
systemctl daemon-reload
systemctl enable --now pactd
systemctl status pactd
```

unit 要点：前台运行 `pactd --config /etc/pact/pact.toml`；`WorkingDirectory=/var/lib/pact`；
`StateDirectory=pact`（`/var/lib/pact`，0700）、`RuntimeDirectory=pact`（`/run/pact`，0700）；
`Restart=on-failure`；`TimeoutStopSec=20s` 覆盖 daemon 的 10s 优雅停机窗口。
加固项包括 `ProtectSystem=strict`、`ProtectHome`、`PrivateDevices`、空 `CapabilityBoundingSet`、
`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6`；daemon 只需写数据目录与运行目录。
日志：`journalctl -u pactd -f`。

#### 2.5.3 容器

```bash
# 构建（仓库根目录；需要 BuildKit，Docker 23+ 默认启用）
# 上下文过滤依赖 dist/Dockerfile.dockerignore，避免把 docs/（含未公开论文）带入构建
docker build -f dist/Dockerfile -t pact:0.1.0 .

# 运行：默认只绑回环；跨主机时启用 [tls] 或前置 TLS 终结，
# 切勿把无 TLS、无身份的明文端口暴露到公网
docker run --rm --name pact \
  -p 127.0.0.1:8787:8787 \
  -v pact-data:/var/lib/pact \
  pact:0.1.0

# 自定义配置（配置里须启用 listen_tcp，容器内默认配置已启用）
docker run --rm -p 127.0.0.1:8787:8787 \
  -v "$PWD/pact.toml:/etc/pact/pact.toml:ro" \
  -v pact-data:/var/lib/pact pact:0.1.0
```

镜像以 uid/gid 10001 的 `pact` 用户运行，`/var/lib/pact` 为数据卷，暴露 `8787`，
`HEALTHCHECK` 每 30s 请求 `/health`。容器内默认配置为 `dist/pact.container.toml`
（`listen_unix=""`，`listen_tcp="0.0.0.0:8787"`）；跨主机访问须在 pactd 启用 `[tls]`（可含 `client_ca` mTLS）
或前置 TLS 终结，并配置 `[auth.tokens]` / `[auth.keys]` / `[auth.acl]`（§2.4、§7.6）。

### 2.6 HTTPS、签名、MCP HTTP 与 OIDC（NFR6 v2）

#### 2.6.1 启用 HTTPS（服务端 TLS / mTLS）

```toml
listen_tcp = "0.0.0.0:8787"
[tls]
certificate = "/etc/pact/server-chain.pem"
private_key = "/etc/pact/server.key"
# client_ca = "/etc/pact/client-ca.pem"   # 加此行即要求客户端证书
```

- TLS 只在 TCP 监听上生效；unix socket 始终明文。配置不完整（缺证书/私钥，或没有 `listen_tcp`）则拒绝启动。
- `client_ca` 存在即启用 mTLS；握手失败的连接被拒绝但 daemon 继续服务。
- 内置客户端只验证服务端，**不出示客户端证书**；需要 mTLS 时请用能出示证书的外部客户端（curl `--cert/--key` 等）。
- 客户端连接：

```bash
pact --tcp pact.example.com:8787 --https --tls-ca ./ca.pem --bearer "$TOKEN" domain list
```

#### 2.6.2 Ed25519 请求签名

```toml
[auth.keys]
architect = "<base64-32B-pubkey>"   # base64 的 32 字节 Ed25519 公钥
```

```bash
# --sign-key：base64 的 32 字节 Ed25519 私钥种子（裸 seed，非 PKCS#8 DER）；--sign-as 必须与请求 participant 一致
pact --tcp 127.0.0.1:8787 --sign-key ./architect.seed --sign-as architect \
  --domain proj --participant architect produce --type design --predecessors <requirement-token-id>
```

- 服务端校验 ±300 秒时间窗与 nonce 防重放；请求体超过 8 MiB 返回 `413`，nonce 重放返回 `429`。
- 签名与 Bearer 可同时配置，但启用签名后除豁免端点外的 REST 请求**必须签名，Bearer 不能替代**；带签名的请求以签名身份为准。签名身份与请求 `participant` 不一致返回 `401`。
- `/health` `/ready` `/metrics` `/mcp` 免验签；启用签名后 `POST /validate` 也需签名。

#### 2.6.3 通过 HTTP 使用 MCP

```toml
[mcp]
enabled = true
public_url = "https://pact.example.com"
```

MCP 客户端配置为 Streamable HTTP：`https://pact.example.com/mcp`，请求头
`Authorization: Bearer <令牌或 OIDC 访问令牌>`。`public_url` 的 authority 会加入 `/mcp` 的 Host 白名单
（同时保留回环）。未配置 OIDC 与令牌时为信任模式，仅限单机/回环，启动会打印警告；
若配置了 `[auth.keys]` 但既无令牌也无 OIDC，daemon 拒绝启动。

#### 2.6.4 用 OIDC 保护 `/mcp`

```toml
[mcp]
enabled = true
public_url = "https://pact.example.com"
[mcp.oidc]
issuer = "https://idp.example.com"
audience = "pact-mcp"
principal_claim = "email"
principal_map = { "alice@example.com" = "alice" }
# discovery = true（默认）或 jwks_uri = "..."
```

- MCP 宿主（或用户）先向 IdP 完成授权并取得访问令牌，再把它直接作为 `Authorization: Bearer` 发给 `/mcp`；daemon 与 CLI 不实现授权码流程，只做资源服务器验证。
- 校验 `iss`/`aud`/`exp`/`nbf`、alg 白名单（RS256/ES256/EdDSA），拒绝 `none`/HS*；claim 映射后的参与者须匹配 `[A-Za-z0-9_-]{1,128}`。
- 发现链：`GET https://pact.example.com/.well-known/oauth-protected-resource` → `authorization_servers` → IdP。401 响应带 `WWW-Authenticate` 指向该元数据。
- IdP/JWKS 不可达时 `/mcp` 返回 `503`；REST、CLI、stdio MCP 不受影响。令牌不缓存、不写日志、不转发给 REST。
- 本地开发可用 `allow_insecure_http = true` 与 `ca = "./idp-ca.pem"`；生产保持默认 `false`。

---

## 3. 写一份域声明

### 3.1 四段文法（§3.1，段序严格）

```pact
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    {test_report + security_report} → deployment   // 多前驱合取
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    reviewer:  reads implementation,
               consumes test_report via implementation,   // 糖形式，脱糖为 when via implementation
               produces test_report when count implementation >= 1;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
```

宽容度与扩展（设计 §3，D16）：段内尾分号可省、`preconditions`/`gates`/`seed_authority` 可为空段
（`types` 至少一项）、`->` 等价 `→`、`//` 行注释、`GateExpr` 优先级 `not > and > or`。
`via T_a` 在解析期脱糖为 `when via T_a`，AST 唯一。

### 3.2 四个注入点（§2.2）

领域知识**只**从这四个地方进入范式，其余部分是领域无关的固定机制：

| 注入点 | 你注入什么 | 对应段 |
|--------|-----------|--------|
| TypeSchema | 有哪些产物类型 | `types` |
| PreconditionGraph | 类型之间的先后依赖 | `preconditions` |
| GateMatrix | 谁能对什么做什么，附加什么条件 | `gates` |
| SeedAuthorityDecl | 谁能无中生有 | `seed_authority` |

可选的第五点是 **Archival Policy**（`AP: Counter → 2^Counter`）：它经正常 Rule-E 执行，
不引入新原语、不引入隐式系统动作。**本实现不内置**（`requirements.md` §6 范围外）；
需要归档策略时，把它实现为一个持有相应 `consumes` gate 的参与者。

### 3.3 良构约束（装载前就会被拒的八类）

| 错误码 | 触发条件 |
|--------|----------|
| `DuplicatePrecondRule` | 同一类型出现两条前置规则（`Precond` 必须是单值函数） |
| `DuplicatePrecondLhs` | 同一规则 LHS 里类型重复，如 `{A + A} → B` |
| `EmptyPrecondLhs` | `{ } → B`，空 LHS |
| `UndeclaredType` | `preconditions` / `gates` / `seed_authority` 引用了 `types` 未声明的类型 |
| `ZeroCountLiteral` | `count T >= 0` 之类非 PosNat 字面量（0 或前导零） |
| `ProduceWithCondition` | `produces T when …`——生产时刻不允许附加条件（§1.5.2） |
| `SourceTypeProduceGate` | 给源类型写 `produces`（源类型只能经 Rule-S 进入，§1.4） |
| `DuplicateGateClause` | 同一 `(参与者, 动作, 类型)` 重复声明 |

解析期就拒绝的两类：空 LHS、非 PosNat 的 count 字面量。
其余在良构层（`validate_domain`）拒绝。**解析返回全部问题，不是首错即停。**

### 3.4 装载前自检

```bash
pact analyze my_domain.pact        # 离线：六项静态分析（§5），不需要 daemon
pact domain validate my_domain.pact # 在线：解析 + 良构 + 装载预演，不留下状态
pact domain load my_domain.pact [--name N]   # 缺省域名 = 文件 stem
```

> `domain validate` 是**在线**命令（走 REST）。没有 daemon 时它会报
> `error [Transport]: client error (Connect)`。想纯离线校验，用 `pact analyze`
> 看首行 `precondition_wf`。

---

## 4. 跑一次流程

### 4.1 唯一正确的动作顺序

```
read（看清楚） → produce --predecessors <前驱 id>（产出新件） → consume（把用尽的移走）
```

颠倒 produce 与 consume 的后果是确定性的：前驱一旦离席，`produce` 立刻报
`PredecessorNotOnCounter`，而且**没有撤销命令**——已经消费的 token 回不来。
示例 §3 第 12 步专门演示了这个陷阱：把顺序换一下，域卡在
`counter=1 archive=3 terminated=false`，既没终止也走不下去。

### 4.2 六条规则对应的命令

```bash
# Rule-S：种子注入（豁免前置，受 SeedAuthority 约束）
pact seed    --domain D --participant external_stakeholder --type requirement \
             [--thing-ref JIRA-123]

# Rule-P：生产（--predecessors 逗号分隔，可多值；类型集合须恰等于 Precond(T)）
pact produce --domain D --participant architect --type design \
             --predecessors 01M500J902XFZ23Q1CWWA3H3S2 [--thing-ref doc://design]

# Rule-R：读取（非破坏）
pact read    --domain D --participant reviewer --token 01M500J9…

# Rule-E：消费（不可逆，入 Archive；可选 --request-id 幂等去重）
pact consume --domain D --participant architect --token 01M500J9… [--request-id RETRY-2F9C]
```

`--thing-ref` 是产物内部结构的**唯一**出口：Token 本身不可变、不透明，
把外部系统的引用（文件路径、工单号、URL）挂在这里。

### 4.3 看清当前局面

```bash
pact counter ls --domain D [--type T]        # 在场 token
pact archive ls --domain D [--type T]        # 已消费 token
pact gates show --domain D --participant P   # 我的全部授权（含条件）
pact enabled    --domain D --participant P   # Rule-W：现在能生产什么 + 见证前驱
pact status     --domain D                   # counter / archive / terminated
```

`enabled` 是 agent 自驱的核心：它返回 `{type, witness_predecessors[]}` 列表，
拿到 witness 就能直接喂给 `produce --predecessors`，不必自己推前置图。

### 4.4 Rule-W：等待与唤醒

引擎被动（D5），不轮询你、不催你。两种自驱方式：

- **订阅推送**：`GET /domains/{d}/events`（SSE）或 MCP `pact_subscribe`；
  Counter 每次变更推一条通知（设计 §6.4）：

  ```json
  { "event": "Produced",        // Seeded | Produced | Consumed | Terminated
    "domain": "se_lifecycle", "seq": 42, "ts": "2026-10-15T14:48:23Z",
    "token_id": "01M500J9…",   // Terminated 时省略
    "summary": { "counter_size": 3 } }
  ```

  MCP 侧经 `notifications/pact/event` 转发。
- **轮询兜底**：`pact enabled` / `pact status`。

`Terminated` 是**一次性**推送。写自驱 agent 时要注意两个已知竞态（设计 §11 阶段 5 的修复记录）：
播种前 Counter 为空会让终止判定平凡为真（应要求 `archive_size > 0` 才承认终止）；
终止推送与轮询判定之间需要宽限窗口排空在途事件。

### 4.5 消费的原子性与幂等

引擎对每一次 Rule-E 的承诺（设计 §8.5、决策 D23）：

- **提交点 = 事件日志落盘（fsync）**。成功返回即意味着该次消费已进入不可变日志；
  其后的索引投影、快照、通知等派生步骤即使失败，也不会回滚、不会改变返回结果。
- **原子域包含 GateHolds 的读集**。`via` / `exists` / `count` / `not` 读到的 Counter 子集，
  与「目标 token 移出 Counter + 写入 Archive」在同一次判定里完成，不存在"检查通过后被人抢先消费"的 TOCTOU。
- **至多一次**。同一 token 二次消费报 `TokenNotOnCounter`；要安全重试请带 `--request-id`：

  ```bash
  pact consume --domain D --participant architect --token ID --request-id RETRY-2F9C
  ```

  同一域内重复的 `request-id` 直接返回**首次**的归档结果（不重复消费），
  因此响应丢失后可以原样重发。该幂等表随事件日志持久化，**跨重启仍生效**。
  同一 `request-id` 却换了 token，会返回 `IdempotencyConflict`（HTTP 409）——那是键复用错误，不是消费失败。

### 4.6 消费与外部动作：物化确认

引擎**不执行**外部副作用：`ThingRef` 对它不透明，它也不校验指向物是否存在。
所以"`consume` 成功"与"外部动作（产物真正生成/落地）成功"是**两个独立承诺**，
二者最终一致而非同一事务——消费成功既不等待、也不回滚外部动作。

要让"产物就位"成为消费的**准入条件**，用 **witness-token 模式**：
外部系统确认就位后经 Rule-S 注入一枚 token（如 `readiness`），
消费方 gate 写成 `consumes work_item when exists readiness`。
于是"就位确认"进入 GateHolds 读集，与消费共享同一次判定；未就位则 `GateConditionFailed` 拦下。
示例见 `examples/domains/external_materialization.pact`。

若选择**先消费、后补做**（saga 式），则由参与者以一次新的 Rule-P 记录"未完成/需重做"事实——
历史不可回滚，补偿是追加的新事实，而不是反向操作。

两种方案**可并用、不互斥**：同一域可对不同入口分别选用（就位可确认的入口用 witness-token，
无法预先确认的入口用 saga 补偿）。

---

## 5. 六项静态分析

`pact analyze <file>`（离线）或 `POST /domains/{d}:analyze`（在线，分析已装载域）或
MCP `pact_analyze`。聚合报告里 **A.5 排首位**，因为良构性是其余五项的前提（Appendix D）：
先修违规，再解读其余结果。

| 分析 | 回答的问题 | 输出 | 阻断装载？ |
|------|-----------|------|-----------|
| A.5 `precondition_wf` | §3.1 良构约束满足吗 | `{well_formed, violations[], warnings[]}` | **是**（violations 非空即拒） |
| A.1 `reachability` | 每个类型能从种子权威类型沿前置图到达吗 | `{seed_types[], reachable[], unreachable[]}` | 否（咨询） |
| A.2 `gate_coverage` | 每个**非源类型**至少有一个 producer 吗 | 无 producer 的类型清单 | 否（咨询） |
| A.3 `consumption_reachability` | 每个类型（**含源类型**）至少有一个 consumer 吗 | 无 consumer 的类型清单 | 否（咨询） |
| A.4 `structural_deadlock` | 前置图里有「无种子入口的强连通分量」吗 | `DeadlockComponent[]` | 否（咨询） |
| A.6 `stranding_risk` | `via` 见证会不会先于依赖它的动作被消费 | `StrandingRisk[]`（带 `potential` 标志） | 否（咨询） |

**退出码**：语法错误或良构违规 → `1`；其余情况（含 unreachable / stranding 等咨询性发现）→ `0`。
咨询性发现不改退出码，是否接受由建模者裁决——CI 里要拦就自己解析 JSON。

三项咨询分析怎么读：

- **A.3 未消费类型**常是「本来就不该有消费者」。示例 `re_pipeline` 的 `baseline` 是常驻配置资产，
  被反复读取而不被耗尽；A.3 只看 GateMatrix 有没有 `consumes` 子句，无法区分「忘了写」和
  「本就不该写」，所以如实报告。也可能是真的技术债（Part II §1.6）。
- **A.4 结构死锁**的两个豁免：SCC 里有成员出现在 `SeedAuthority`（可点火），
  或存在来自 SCC 外部且种子可达的入边（可馈入）。两者都不满足才报。
- **A.6 搁浅风险**的 `potential` 字段：前置图**有环**时为 `true`（保守报告
  `POTENTIAL_STRANDING`），**无环**时为 `false`（确定性报告 `STRANDING_RISK`）。
  只有 `via` 引用的类型是**直接**前驱时才有风险；间接祖先的 `via` 永不可满足，不算风险。

### 5.1 术语陷阱：两个「源类型」

同一份 `analyze` 输出里，`reachability` 与 `gate_coverage` 说的**不是同一个东西**：

| 出处 | 实际含义 | JSON 键名 |
|------|----------|-----------|
| `reachability: N 源类型` | **SeedAuthority 里的类型**（`{T' : ∃p, (p,T') ∈ SeedAuthority}`） | `seed_types` ✓ 准确 |
| `gate_coverage: N 个无 producer 的非源类型` | **§1.4 源类型**（`Precond(T) = ∅`）被排除 | 按 `is_source_type` 判定 ✓ |

CLI 的人类可读输出把前者打印成「源类型」，是措辞不准。`security_review.pact` 单独分析是活证据：

```
reachability: 0 源类型，0 可达，2 不可达
gate_coverage: 0 个无 producer 的非源类型
```

`gate_coverage` 报 0，正因为 `implementation` 的 `Precond` 是空集、按 §1.4 **是**源类型而被排除；
`reachability` 报「0 源类型」，则因为该片段没有 `SeedAuthority`。**机读一律以 JSON 键名为准。**

### 5.2 一个警告：`SEED_PRODUCE_OVERLAP`

A.5 的 warnings 报「某参与者对非源类型 T 持 seed 权威，且 T 存在 produces gate」。
这在规范上是**安全**的（seed 实例可作前驱被消费，produce 路径产出新实例），
但通常是建模混淆或需要显式部署裁决的信号，应复核意图。

实现是**类型级**判定（produces gate 取全体参与者的并集，不要求与 seed 持有者同一参与者），
比 §1.4.3「同一参与者」的表述更宽——跨参与者重叠也告警。偏差方向是假阳性、不漏报，
登记为规范的安全超集（设计 D21）。

---

## 6. 片段组合

大域可以拆成片段并行开发，再组合。`examples/fragments/` 下有一对演示片段
（`security_review.pact` + `security_blocking.pact`）。

### 6.1 四条组合条件（§1.8）

| 条件 | 要求 | 冲突码 |
|------|------|--------|
| C1 | 前置唯一性保持：合并后每个类型仍只有一条前置规则 | `PreconditionMismatch` |
| C2 | 共享类型的前置一致 | `PreconditionMismatch` |
| C3 | 共享 `(参与者, 动作, 类型)` 的 gate 条件必须**语法同一** | `GateConditionMismatch` |
| C4 | 合并结果仍须通过 `validate_domain` | `IllFormed`（内含 §3.4 的 `DomainError` 变体） |

C1+C2 的判定要点：**只有双方都为同一 rhs 声明了前置规则且集合不同**才报冲突。
**单侧声明是合法的**（D18）——缺席方不构成约束，合并取并集，这是 §1.8 示例的语义。

### 6.2 用法

片段是**挂在宿主域上的增量**，不是彼此独立拼装的两半——组合命令的第一个参数通常是完整宿主域。

成功合并（给 `se_lifecycle` 挂一条安全扫描支线；下为实测输出，`gates` 段有删节，退出码 0）：

```bash
pact compose examples/domains/se_lifecycle.pact examples/fragments/security_review.pact
```

```
types { deployment; design; implementation; requirement; security_report; test_report }

preconditions {
    test_report → deployment;
    requirement → design;
    design → implementation;
    implementation → security_report;
    implementation → test_report;
}

gates {
    …（八个参与者的 gate 子句按参与者名排序合并）
    security_scanner: reads implementation, produces security_report;
    …
}

seed_authority {
    external_stakeholder: seeds requirement;
}
```

C2 冲突被机械拦下（同一宿主域，换一个对 `deployment` 前置理解不同的片段，退出码 1）：

```bash
pact compose examples/domains/se_lifecycle.pact examples/fragments/security_blocking.pact
```

```
error [ComposeConflict]: 片段组合冲突（1 处）
```

`--json` 给出可机读的冲突明细（`detail.conflicts[]` 是 `Conflict` 变体的 Debug 串）：

```json
{"error":{"code":"ComposeConflict","detail":{"conflicts":["PreconditionMismatch { ty: TypeName(\"deployment\"), pre_a: {TypeName(\"test_report\")}, pre_b: {TypeName(\"security_report\"), TypeName(\"test_report\")} }"]},"message":"片段组合冲突（1 处）"}}
```

读法：宿主认为 `deployment` 只需 `test_report`，片段认为还需 `security_report`——这就是 C2 要拦的分歧。

每个输入片段会被**单独**跑一遍良构校验（含类型闭包），所以**片段必须自足声明它引用的全部类型**。
组合是离线操作，不需要 daemon。

### 6.3 CLI 与 MCP/REST 的语义差异（重要）

| 接口 | 行为 |
|------|------|
| CLI `pact compose f1 f2 …` | 只把合并后的域声明文本**打印到 stdout**，不装载 |
| REST `POST /domains/{name}:compose` | 调 `compose_and_load`，**装载为命名域**，返回 `DomainSummary` + `201` |
| MCP `pact_compose` | 同 REST，**会装载** |

也就是说 MCP 的 `pact_compose` 是有副作用的。想只看不装，用 CLI。

---

## 7. 审计、事件溯源与重放

### 7.1 存储布局（设计 §7.2）

```
<data_dir>/
  <domain>/                        # 每个域声明一个子目录
    <instance>.events.jsonl        # 实例级真相源，只追加
    <instance>.snapshot.json       # 定期 / 压缩时写入的快照（自足锚），原子写（tmp + rename）
    <instance>.logmeta.json        # 压缩水位（base_seq，缺省不存在）
    declaration.pact               # 声明原文（启动恢复时回填声明注册表）
  index.tursodb                    # 全部实例共用的查询索引（派生物，(domain, instance) 双列）
  index.tursodb-wal
  pactd.lock                       # data_dir 独占 flock（单写者护栏）
```

**JSONL 是真相源，SQLite 索引与快照都是派生物。** 索引丢了会重建；JSONL 中部损坏则 fail-fast。

**实例布局（D27）**：实例名作为**路径段**承载，默认实例名为 `default`——`<domain>/default.events.jsonl` 即域的默认实例。顶层遗留的单层文件（`<domain>.events.jsonl` / `<domain>.snapshot.json`）**不再被识别**：启动恢复时直接 fail-fast（`LegacyLayoutDetected`），须人工迁移，不静默丢数据。同一 `data_dir` 上 daemon 以 `<data_dir>/pactd.lock` 独占 flock 保证单写者，第二个实例启动即被拒。

### 7.2 事件类型（设计 §7.1）

```rust
DomainLoaded { name, declaration }   // 声明原文存档，可复现解析
Seeded     { seq, ts, token }
Produced   { seq, ts, token }        // token 内含 depends_on / produced_by
Consumed   { seq, ts, token, by, request_id? }  // token 全量自足；request_id 可选幂等键（D23）
Terminated { seq, ts }               // Rule-T 检测结果，信息性
```

`seq` 全局单调。**Rule-R 不持久化**（D9：读取无状态变更，重放不需要）——
所以「谁读过什么」不在事件日志里。需要记录时可开启 `read_audit = true`（§7.5），
审计写入独立文件、不改事件日志；未开启时需在外层自行记录。

### 7.3 离线重放

```bash
pact replay <data_dir>/se_lifecycle/default.events.jsonl
# → replayed 10 events (domain se_lifecycle): counter=0 archive=5
```

不需要 daemon。重放结果与线上状态逐字段等价（集成测试锚定）。
崩溃恢复走同一条路径：最新快照 + 其后的事件重放；JSONL 末行半截（写入中途崩溃）可被安全跳过续作。

**事件 schema 版本**：每行事件带顶层 `version`（当前 `1`，如 `{"version":1,"kind":"Seeded",…}`）。
缺 `version` 的旧日志按 v1 重放（向后兼容）；`version` 高于当前支持版本时 **fail-fast
（`UnsupportedSchemaVersion`）**，不会静默按 v1 误读——升级 pactd 后旧数据可直接用，降级则须先迁移。

### 7.4 日志压缩（D25）

事件日志只追加，长期运行会无限增长。压缩 = **写全量快照后截断已覆盖的前缀**：

```bash
pact compact --domain D                      # 丢弃全部已入快照的前缀（keep_last = 0）
pact compact --domain D --instance I --keep-last 100   # 保留最近 100 条，便于审计
```

- 压缩后日志可能不再包含 `DomainLoaded`——恢复改用**快照为锚**（快照内置声明原文、
  `dl_seq`、Counter/Archive 状态与 Rule-E 幂等表），旧前缀内被丢弃的 `Consumed` 行
  不会破坏「同 `request_id` 重试返回首次结果」。
- 水位记在 `<instance>.logmeta.json`（`base_seq`），**单调不回退**；重复压缩不会让 `seq` 重号。
- **数据安全**：若日志已被截断但快照缺失/不覆盖水位，启动直接 fail-fast
  （`SnapshotRequiredForCompactedLog`），不静默丢数据。
- **代价**：被丢弃前缀**不再可从日志审计**——审计视图退化为「快照 + 保留的尾部事件」。
  需要完整历史时保留 `keep_last` 足够大，或先备份 JSONL 再压缩。
- **安全边界**：压缩是**破坏性**运维操作；启用 `[auth.acl]` 时与其它实例端点一样按 `(domain, instance)` 授权；未启用 ACL 时不做域级授权（但启用签名时仍需签名）——只应在 unix socket / 回环 / 受信网络暴露。另注意 `compact_on_start = true` 且 `compact_keep_last = 0` 时，**每次重启都会丢弃全部已入快照的审计前缀**；需要留存审计时务必显式设置 `compact_keep_last`（pactd 启动会打印告警）。
- 可选启动自动压缩：`compact_on_start = true` + `compact_keep_last = N`；
  恢复后逐实例压缩，单个实例失败只告警、不阻断 daemon 启动。
- REST 等价入口：`POST /domains/{d}[/instances/{i}]/compact {"keep_last": 0}`，
  返回 `{domain, instance, snapshot_seq, base_seq, retained_events, discarded_events}`；MCP 为 `pact_compact`.

### 7.5 读审计视图（D25）

`read_audit = true` 开启后，每次 Rule-R 读取成功时会在独立审计流追加一条记录（`version/ts/domain/instance/participant/token_id`），审计流与主事件日志分离，不影响 seq / 重放 / Rule-T。

查看最近 `limit` 条（默认 100，上限 1000）：

```bash
curl --unix-socket ./pact.sock http://localhost/domains/proj/read-audit?limit=20
curl --unix-socket ./pact.sock http://localhost/domains/proj/instances/foo/read-audit?limit=100
```

响应格式：`{"records": [{"version":1,"ts":"...",...}, ...]}`。

- **fail-closed**：开启审计后，如果审计写入失败，本次读取以 500 拒绝，不返回 token。
- **ACL**：读审计视图和其它实例端点一样受 `[auth.acl]` 约束。
- **持久化**：审计流不参与快照与压缩，运维可按需轮转归档。

### 7.6 域级 ACL（D25）

`[auth.acl]` 非空即启用域级授权，必须同时配置 `[auth.tokens]` 或 `[auth.keys]`（至少其一）。格式：

```toml
[auth.tokens]
architect = "token-arch"
reviewer = "token-rev"

# 或者/同时使用签名身份：
# [auth.keys]
# architect = "<base64 32B 公钥>"

[auth.acl]
architect = ["proj/*"]          # 通配：proj 下任意实例都允许
reviewer = ["proj/default"]    # 仅允许 reviewer 读 proj/default
```

授权规则（除 `/health` `/ready` `/metrics` `/mcp` `/validate` 均生效）：

- `load` / `instantiate` / `unload` / `compact` / `:analyze` / `:compose`：启用签名时取签名身份，否则取 Bearer 令牌归属的参与者，按 `(domain, instance)` 授权。
- 四类动作（seed / produce / read / consume）：请求体/查询参数的 `participant` 必须与启用签名时的签名身份（或未启用签名时的 Bearer 令牌归属）一致，且参与者对实例有授权。
- 查询（counter / archive / gates / enabled / status / events / read-audit）：启用签名时取签名身份，否则取 Bearer 令牌归属的参与者，按 `(domain, instance)` 授权。
- `GET /domains`：按参与者过滤，只返回其有权访问的实例（空域剔除）。

未授权返回 `403 Forbidden`（结构化错误体，含 `domain` `instance`）；无有效身份/身份与 participant 不一致返回 `401 Unauthorized`。未配置 `[auth.acl]` 保持原有信任声明行为（即任何人说自己是谁就是谁，不做域级授权）。

### 7.7 CPN 模型生成（离线，阶段 6）

把声明文件编码为 CPN Tools 可加载的彩色 Petri 网，用于可视化与形式验证：

```bash
pact cpn examples/domains/se_lifecycle.pact --out se_lifecycle.cpn
pact cpn my.pact --out my.cpn --instance project_a
pact cpn my.pact --out my.cpn --bound task=3 --bound artifact=2
```

- **单实例作用域**：一次只编码一个 Counter 实例；库所/迁移统一带 `i_<instance>__` 前缀，不同实例的模型互不共享。`--instance` 只影响命名，不读取运行期状态。
- **结构映射**：每个类型一个库所 + Archive；token 颜色含 `TokenId / Participant / ThingRef / PrecondRefs / Timestamp`；生产迁移用 test arc 保留前驱，读迁移自环，消费迁移把 token 移入 Archive，种子迁移无输入弧。
- **条件**：`and` / `or` / `not` 会展开；`or` 生成多条迁移（名称 `_c1/_c2…`）；`not` 与 `count T = n` 需要互补库所，因此必须为涉及的每个类型提供真实并发上界 `--bound T=N`。上界不能从声明自动推导；给错会导致模型行为不等价。缺上界时命令失败并提示类型。
- **不支持**：`not via T`（见证是依赖关系而非计数，无法用互补库所表达）会显式报错；请改写条件或人工建模。
- **恒假条件**：仍保留迁移并置 `guard=false`，便于和声明条目逐一对账。
- **验证**：输出为 CPN Tools XML（format 6），已通过 CPN DTD 校验与结构化测试。CPN Tools GUI 的加载与状态空间实验需人工验收；沙箱内只做 XML/DTD 校验。

---

## 8. 接口参考

### 8.1 CLI

全局 flag（对所有子命令生效）：

| flag | 说明 |
|------|------|
| `--unix <PATH>` | unix socket 路径，缺省 `./pact.sock` |
| `--tcp <ADDR>` | TCP 地址（如 `127.0.0.1:8787`）；**设置时优先于 `--unix`** |
| `--https` | 以 TLS 连接 `--tcp`（须同时给 `--tcp`）；服务端须启用 `[tls]` |
| `--tls-ca <PATH>` | 信任的服务端自定义 CA（PEM）；IdP 的 CA 在 daemon 侧 `[mcp.oidc].ca` 配置 |
| `--sign-key <PATH>` | Ed25519 私钥种子文件（base64 32 字节）；须与 `--sign-as` 成对 |
| `--sign-as <PARTICIPANT>` | 签名身份名；须与请求 `participant` 一致 |
| `--bearer <TOKEN>` | 参与者令牌（`Authorization: Bearer`）；信任声明模式下可省 |
| `--bearer-file <PATH>` | 从文件读取令牌（与 `--bearer` 互斥，便于 secret 挂载） |
| `--json` | 以 §6 线上格式输出 JSON |

`--sign-key` 与 `--sign-as` 必须同时出现；只给一个会在建立客户端前报用法错误。

子命令（**在线** = 必须有 daemon；**离线** = 纯函数层，不经 daemon）：

| 命令 | 归属 | 说明 |
|------|------|------|
| `daemon start [--config PATH]` | — | 前台启动 daemon，缺省配置 `pact.toml` |
| `domain validate <FILE>` | 在线 | 解析 + 良构 + 装载预演，不留状态 |
| `domain load <FILE> [--name N] [--instance I] [--force]` | 在线 | 装载/替换命名域（默认实例 `default`），缺省名 = 文件 stem |
| `domain list` | 在线 | 已装载域清单（打印 `name [instances…]`） |
| `domain info --domain D [--instance I]` | 在线 | 域信息 + 状态摘要 |
| `domain unload --domain D [--instance I]` | 在线 | 卸载（内存态；事件日志保留） |
| `domain instantiate --domain D --instance I` | 在线 | 在同一域声明下**新建实例**（D27） |
| `domain instances --domain D` | 在线 | 打印该域的实例清单 |
| `seed --domain D --participant P --type T [--thing-ref R] [--instance I]` | 在线 | Rule-S |
| `produce --domain D --participant P --type T [--predecessors a,b] [--thing-ref R] [--instance I]` | 在线 | Rule-P |
| `read --domain D --participant P --token ID [--instance I]` | 在线 | Rule-R |
| `consume --domain D --participant P --token ID [--instance I]` | 在线 | Rule-E |
| `counter ls --domain D [--type T] [--instance I]` | 在线 | Counter 查询 |
| `archive ls --domain D [--type T] [--instance I]` | 在线 | Archive 查询 |
| `gates show --domain D --participant P [--instance I]` | 在线 | 我的 gate 子句（含条件） |
| `enabled --domain D --participant P [--instance I]` | 在线 | Rule-W：当前可执行的生产 |
| `status --domain D [--instance I]` | 在线 | Counter 概要 + `terminated` |
| `compact --domain D [--instance I] [--keep-last N]` | 在线 | 日志前缀压缩（D25；缺省 `--keep-last 0` = 丢弃全部已入快照的前缀） |
| `analyze <FILE>` | **离线** | 六项静态分析 |
| `compose <FILES…>` | **离线** | 片段组合（≥2 个），打印合并文本 |
| `replay <FILE>` | **离线** | 重放 `<instance>.events.jsonl`（校验事件 schema `version`） |
| `cpn <FILE> --out PATH [--instance I] [--bound T=N]…` | **离线** | 生成单实例 CPN Tools 模型（论文 §4.5/§B；负条件需上界） |

**实例 flag（D27）**：所有域操作命令的 `--instance I` 可选，**缺省 = 默认实例 `default`**——既有不带该 flag 的调用与脚本语义不变。`--force` 仅用于对**非空实例**（`last_seq > DomainLoaded.seq`）的再加载：默认拒绝（`DomainBusy`），加 `--force` 才重置其 Counter。

输出与退出码：成功 → `0`；失败 → `1`，人类模式把 `error [<code>]: <message>` 写 **stderr**，
`--json` 模式把 §6.3 错误体写 **stdout**。

### 8.2 REST

监听 unix socket（默认）与/或 TCP（可 TLS/mTLS）。全部载荷取 §6 线上格式。
身份：静态 Bearer 或 Ed25519 签名；启用 `[auth.keys]` 后除运维三端点、`/mcp` 与 OIDC 元数据外的请求均需签名（Bearer 不能替代）。OIDC **不用于 REST**。

| 方法与路径 | 请求体 | 成功响应 |
|-----------|--------|----------|
| `POST /validate` | `{declaration}` | `200` `{ok, types[], participants[], gates, seed_authority}` |
| `POST /domains` | `{name, declaration[, instance][, force]}` | `201` `DomainSummary{name, instance, types, gates, seed_authority, last_seq}`（declare + instantiate，缺省实例 `default`） |
| `GET /domains` | — | `200` `{domains: [{name, instances[]}]}` |
| `POST /domains/{d}/instances` | `{instance}` | `201` `DomainSummary`（在同一域声明下新建实例，D27） |
| `GET /domains/{d}` | — | `200` `{name, counter_size, archive_size, terminated}`（`default` 实例） |
| `DELETE /domains/{d}` | — | `204` |
| `POST /domains/{d}:analyze` | — | `200` `AnalysisReport`（六项，A.5 首位） |
| `POST /domains/{d}:compose` | `{fragments: [...]}` | `201` `DomainSummary`（**会装载**到 `default` 实例） |
| `POST /domains/{d}/seeds` | `{participant, type, thing_ref?}` | `201` `{token}` |
| `POST /domains/{d}/tokens` | `{participant, type, predecessors[], thing_ref?}` | `201` `{token}` |
| `POST /domains/{d}/tokens/{id}/read` | `{participant}` | `200` `{token}` |
| `POST /domains/{d}/tokens/{id}/consume` | `{participant}` | `200` `{entry}` |
| `GET /domains/{d}/counter?type=` | — | `200` `{tokens: [...], size}` |
| `GET /domains/{d}/archive?type=` | — | `200` |
| `GET /domains/{d}/gates/{participant}` | — | `200` `{gates: [...]}` |
| `GET /domains/{d}/enabled?participant=` | — | `200` `{options: [{type, witness_predecessors[]}]}` |
| `GET /domains/{d}/status` | — | `200` `{counter_size, archive_size, terminated}` |
| `POST /domains/{d}/compact` | `{keep_last?}`（缺省 0） | `200` `{domain, instance, snapshot_seq, base_seq, retained_events, discarded_events}`（D25） |
| `GET /domains/{d}/events` | — | SSE 流（含 keep-alive），每条 `data:` 是 §6.4 通知 |
| `GET /domains/{d}/read-audit?limit=` | — | `200` `{records:[...]}`（D25 子项 3；缺省 100、上限 1000） |
| `POST/GET/DELETE /mcp` | MCP JSON-RPC / 会话控制 | MCP Streamable HTTP（D35；`[mcp].enabled = true` 时存在） |
| `GET /.well-known/oauth-protected-resource` | — | `200` `{resource, authorization_servers, bearer_methods_supported}`（D36；OIDC 未启用时 `404`） |

**签名错误语义（D34）**：缺头/未知参与者/时间戳越界/签名错误 → `401`；请求体 > 8 MiB → `413`；nonce 重放 → `429`。`/mcp` 的 OIDC 令牌非法 → `401`，IdP/JWKS 不可达 → `503`。

**域级 ACL（D25 子项 3，可选）**：配置 `[auth.acl]` 后，上表全部域作用域端点（含实例镜像组与 `read-audit`）按 `(domain, instance)` 授权，无有效身份/身份不匹配 `401`、越权 `403`；`GET /domains` 按授权过滤。未配置时不做域级授权，但仍受签名/令牌中间件约束。详见 §7.6。

**运维端点（D25，免身份）**：`GET /health`（存活，`{status:"ok"}`）、`GET /ready`（就绪，引擎不可应答时 `503`）、`GET /metrics`（Prometheus 文本格式）。三者仅供本地/内网探活与抓取，**不校验身份**、也不用 §6 线上格式（详见 §2.3.1）。**`POST .../compact`** 在未启用 ACL 时不做域级授权（但启用签名时仍需签名；破坏性运维操作，会丢弃已入快照的审计前缀），启用 ACL 后按实例授权，暴露边界见 §2.3.1。

**实例路由（D27）**：上表 `/domains/{d}/…` 的语义 = **`default` 实例**。实例级为 `/domains/{d}/instances/{i}/…` 镜像组——seeds / tokens / tokens/{id}/read / tokens/{id}/consume / counter / archive / gates/{participant} / enabled / status / compact / events / read-audit 全套同形。实例名谓词 `[A-Za-z0-9_-]{1,128}`（同域名），非法名返回 `400`。

实现注记：`{d}:analyze` / `{d}:compose` 因路由库不支持同段「参数 + 字面量」，
在 `/domains/{d}` 统一入口内按段内冒号后缀分发，**对外 URL 不变**。

### 8.3 MCP（stdio 与 Streamable HTTP）

```bash
pact-mcp --unix <sock> [--bearer <token>] [--sign-key <seed> --sign-as <P>]   # 或 --tcp
pact-mcp --tcp <addr> --https [--tls-ca <ca.pem>] [--bearer <token>|--bearer-file <path>] \
         [--sign-key <seed> --sign-as <P>]
```

stdio 形态由 agent 拉起为子进程，工具经 REST 转发至 daemon；`--https/--tls-ca/--sign-key/--sign-as/--bearer-file` 与 CLI 同名（`--bearer-file` 与 `--bearer` 互斥）。schema 由 serde + schemars 派生；错误以 §6.3 结构化 JSON 返回（`CallToolResult.is_error = true`），agent 可编程处理（例如 gate 被拒后转入等待）。

远程形态无需 `pact-mcp`：MCP 客户端直接连接 daemon 的 `https://<public_url>/mcp`，以 `Authorization: Bearer <静态令牌或 OIDC 访问令牌>` 认证（OIDC 见 §2.6.4）。两种形态工具、参数与错误语义完全一致。

| 工具 | 语义 |
|------|------|
| `pact_domain_validate` | 校验声明文本良构，返回结构概要（不加载） |
| `pact_domain_load` | 加载/替换命名域（幂等按 name；可选 `instance` / `force`） |
| `pact_domain_instantiate` | 在同一域声明下**新建实例**（D27，可选 `instance`） |
| `pact_list_domains` | 已加载域清单（每域含 `instances[]`） |
| `pact_analyze` | 六项静态分析聚合报告（A.5 前置） |
| `pact_compose` | 组合多片段**并加载为命名域**（有副作用，见 §6.3；作用于 `default` 实例） |
| `pact_seed` | Rule-S |
| `pact_produce` | Rule-P（显式传前驱 token id 列表） |
| `pact_read` | Rule-R（返回 token 全量数据，不改 Counter） |
| `pact_consume` | Rule-E（移出 Counter，归档） |
| `pact_counter_query` / `pact_archive_query` | 状态查询（可按类型过滤） |
| `pact_my_gates` | 参与者的全部 gate 子句（含条件），agent 自省 |
| `pact_enabled` | Rule-W 轮询：当前可满足的 Rule-P 组合 |
| `pact_status` | Counter 概要 + `terminated` 判定 |
| `pact_compact` | 日志前缀压缩（D25；`keep_last` 缺省 0） |
| `pact_subscribe` | 注册 Counter 变更通知；事件经 `notifications/pact/event` 推送 |

共 **17** 个工具。除 `pact_domain_validate` / `pact_analyze` / `pact_compose` / `pact_list_domains` 外，全部工具接受可选 `instance` 参数（缺省 = `default` 实例，D27）。

### 8.4 pact-runtime（生产编排）连接参数

`pact-runtime --config <toml> [--https] [--tls-ca <pem>] [--sign-key <seed>] [--sign-as <P>]`；TOML 的 `endpoint` 支持 `https = true` 与 `tls_ca`，签名身份只经命令行 `--sign-key/--sign-as` 提供（作用于 runtime 的 REST 客户端）。

`Endpoint` 提供 `unix(path)`、`tcp(addr)` 与 `https(addr, ca)` 三种构造；`https` 等价于 `tcp` + TLS。runtime 的 stdio MCP 子进程目前只透传 TLS 与 Bearer，不透传签名参数；连接签名 daemon 时请使用 REST 通道或 TLS + Bearer。

---

## 9. 错误码表

线上错误体统一为（§6.3）：

```json
{ "error": { "code": "SeedAuthorityDenied",
             "detail": { "participant": "architect", "ty": "requirement" },
             "message": "(architect, requirement) ∉ SeedAuthority：无种子注入权" } }
```

`code` = 枚举变体名，**机器可判**；`detail` 携带结构化字段；`message` 面向人类。
被拒动作一律整体失败，**无部分效果**。

### 9.1 规则层（HTTP 409 Conflict）

| code | 含义 |
|------|------|
| `StaticGateDenied` | `(参与者, 动作, 类型)` 不在 GateMatrix 里 |
| `GateConditionFailed` | gate 存在但其 `GateExpr` 在当前 Counter 上不成立（如 `via` 见证已被消费） |
| `PreconditionMismatch` | 提供的前驱类型集合 ≠ `Precond(T)` |
| `SourceTypeProductionDenied` | 试图 produce 源类型（只能经 Rule-S） |
| `PredecessorNotOnCounter` | 前驱 token 不在 Counter 上（多半已被消费） |
| `DuplicatePredecessor` | 前驱列表里有重复 token id |
| `TokenNotOnCounter` | read/consume 的目标 token 不在 Counter 上 |
| `SeedAuthorityDenied` | `(参与者, 类型) ∉ SeedAuthority` |

### 9.2 声明层（HTTP 400 Bad Request）

`code = "Syntax"`（`detail.errors[]`，含行列位置与期望符号）或
`code = "WellFormedness"`（`detail[]` 是 §3.4 八类 `DomainError` 变体的结构化形式）。
两者都**一次返回全部问题**。

### 9.3 组合层（HTTP 409 Conflict）

`code = "ComposeConflict"`，`detail.conflicts[]` 是冲突的 Debug 串，形如：

```
PreconditionMismatch { ty: TypeName("deployment"),
  pre_a: {TypeName("test_report")},
  pre_b: {TypeName("security_report"), TypeName("test_report")} }
```

### 9.4 接口与运行时层

| code | HTTP | 触发 |
|------|------|------|
| `IdempotencyConflict` | 409 | 同一 `request_id` 已用于另一 token 的消费（复用幂等键，D23） |
| `InstanceNotFound` | 404 | 实例未加载（`domain/instance` 不存在或未 `instantiate`，D27） |
| `DomainNotDeclared` | 404 | 目标域无声明登记（实例化前须先 declare/load） |
| `InstanceExists` | 409 | 实例已存在（重复 `instantiate`） |
| `DomainBusy` | 409 | 对**非空实例**（`last_seq > DomainLoaded.seq`）再加载；须显式 `force` 才重置 |
| `InvalidDomainName` | 400 | 域名不匹配 `[A-Za-z0-9_-]{1,128}`（防路径穿越） |
| `InvalidInstanceName` | 400 | 实例名不匹配 `[A-Za-z0-9_-]{1,128}`（防路径穿越） |
| `Unauthorized` | 401 | 令牌/签名缺失或不匹配；签名身份与请求 `participant` 不一致；`/mcp` 的 OIDC 令牌缺失/非法/claim 不合规 |
| `Forbidden` | 403 | ACL 拒绝：身份有效但无该 `(domain, instance)` 授权 |
| `PayloadTooLarge` | 413 | 签名验证前的请求体超过 8 MiB（D34） |
| `TooManyRequests` | 429 | 签名 nonce 重放，或 nonce 缓存已满（D34） |
| `NotFound` | 404 | 路径不存在；OIDC 未启用时 `/.well-known/oauth-protected-resource` 亦返回 404 |
| `MethodNotAllowed` | 405 | 方法与路径不匹配（如 `GET /domains/{d}:analyze`） |
| `BadRequest` | 400 | JSON 请求体解析失败 |
| `Store` | 500 | 存储写入失败；**含 `UnsupportedSchemaVersion`**（日志/快照 `version` 高于当前支持版本）、**`SnapshotRequiredForCompactedLog`**（日志已截断但快照缺失/不覆盖水位）、**顶层遗留单层文件**（`LegacyLayoutDetected`，须人工迁移） |
| `Recovery` | 500 | 启动恢复失败 |
| `EngineStopped` | 503 | 引擎已停止 |
| `ServiceUnavailable` | 503 | IdP 发现/JWKS 拉取不可达（仅 `/mcp`，D36） |
| `Transport` | —（仅 CLI/MCP） | 连不上 daemon：`client error (Connect)` |
| `Io` | —（仅 CLI） | 读取 `.pact` / `.jsonl` 文件失败，`detail.path` 是路径 |
| `Usage` | —（仅 CLI） | 命令用法错误，如 `compose` 少于两个片段 |
| daemon 启动拒绝 | —（进程退出） | `data_dir` 已被另一 daemon 独占（`pactd.lock` flock 失败，D27） |

---

## 10. 故障排查

| 症状 | 原因 | 处置 |
|------|------|------|
| `error [Transport]: client error (Connect)` | daemon 没起，或 `--unix`/`--tcp` 指错 | 起 daemon；确认 socket 路径与配置一致 |
| 在线命令都失败，`analyze` 却正常 | 你在用离线命令的预期跑在线命令 | 见 §8.1 归属列；`domain validate` 是**在线**的 |
| `PredecessorNotOnCounter` | 前驱已被消费（顺序错了） | 无撤销；`domain unload` + `load` 重来，或重新 seed |
| 装载后 `terminated=true` | Counter 为空即平凡满足 Rule-T | 正常。`seed` 之后翻回 `false` |
| `GateConditionFailed: … Via(TypeName("x"))` | `via x` 的见证 token 已被消费 | 调整消费顺序，或改用不带 `via` 的 gate |
| `IdempotencyConflict` | 复用了别的消费已经用过的 `request_id` | 换一个新的幂等键重发；同一键只对应一次消费（D23） |
| `SourceTypeProduceGate` | 给 `Precond = ∅` 的类型写了 `produces` | 改成 `seed_authority`，或给它加前驱 |
| `pact compose` 报 `UndeclaredType` | 片段引用了未在自己 `types` 段声明的类型 | 片段必须自足（§6.2） |
| 停不掉 daemon | 没有 `daemon stop` | `kill <pid>`（§2.3） |
| 索引文件损坏/丢失 | `index.tursodb` 是派生物 | 删掉重启，会按 JSONL 重建 |
| `analyze` 说 `unreachable` 但域能跑 | 片段单独分析时缺少 `SeedAuthority` | 组合后再分析（示例 §1.2） |
| 启动报 `Store` / `LegacyLayoutDetected` | `data_dir` 顶层有旧版单层文件（`<domain>.events.jsonl`） | 手工迁移到 `<domain>/default.events.jsonl` 后重启（§7.1，不自动迁移） |
| `DomainBusy` | 对非空实例再 `load`（会重置 Counter） | 确认要替换则加 `--force`（CLI）/ `force: true`（REST）；否则换实例名 |
| daemon 启动即退出（锁冲突） | 同一 `data_dir` 已有另一 daemon | 停掉旧实例，或换 `data_dir`（`pactd.lock` 独占，§7.1） |
| 启动报 `UnsupportedSchemaVersion` | 数据由更新版 pactd 写入（事件 schema 版本更高） | 升级 pactd 或按新版格式迁移；**不可**期望旧版静默重放（§7.3） |
| 启动报 `SnapshotRequiredForCompactedLog` | 日志已被压缩截断，但快照缺失/未覆盖 `base_seq` | 从备份恢复 `<instance>.snapshot.json`；勿手工删除快照（§7.4） |
| 压缩后早期事件查不到 | 前缀已随快照丢弃（预期取舍） | 需要完整历史时先备份 JSONL，或用 `--keep-last` 保留足够尾部后重压（§7.4） |
| `error [Transport]: client error (Connect)` 且使用 `--https` | 服务端未启用 `[tls]`，或客户端没给 `--tls-ca` | 确认 `listen_tcp` + `[tls]` 完整；自签/私有 CA 用 `--tls-ca`（§2.6.1） |
| TLS 握手失败（mTLS） | 服务端配置了 `client_ca`，客户端未出示证书 | 内置 CLI/MCP/runtime 不出示证书；改用外部客户端 `--cert/--key`，或移除 `client_ca`（§2.6.1） |
| 401 且提示「时间戳」/「nonce」/「签名」 | 客户端时钟偏差 > 300s、nonce 重放或签名种子不匹配 | 校准时间；每次请求重新签名；确认 `--sign-key` 与 `[auth.keys]` 对应（§2.6.2） |
| 413 `PayloadTooLarge` | 签名请求体 > 8 MiB | 减小载荷；该上限在验签前生效（设计 §9.6） |
| 429 `TooManyRequests` | 同一 nonce 被重放，或 300s 内 nonce 缓存达到 8192 | 勿重放同一请求；高并发部署缩短重试间隔或拆 daemon（设计 §9.6） |
| `/mcp` 401 | 未带令牌、JWT 过期/aud 不符/claim 映射失败 | 检查 `Authorization: Bearer`；对照 `audience`、`principal_claim`、`principal_map`（§2.6.4） |
| `/mcp` 503 | IdP 发现或 JWKS 不可达 | 检查 `issuer`、网络、`ca`/`allow_insecure_http`；REST 不受影响（§2.6.4） |
| `/mcp` 403 或 Host 被拒 | `public_url` 的 authority 不在允许列表，或请求 Host 不匹配 | 设置正确的 `public_url`；客户端使用该主机名访问（设计 §9.7） |
| `/mcp` 信任模式告警 | 未配置 OIDC、未配置令牌表 | 仅限单机/回环；生产请配置 `[auth.tokens]` 或 `[mcp.oidc]`（§2.6.3） |

---

## 11. 已知偏差与限制

如实登记，均不影响范式语义：

1. **CLI 措辞**：`reachability` 那行把 `seed_types` 打印成「源类型」，与同一份输出里
   `gate_coverage` 用的 §1.4 源类型定义冲突（§5.1）。机读以 JSON 为准。
2. **MCP 工具描述**：`pact_analyze` 的 description 写「五项静态分析」，实际是六项 A.1–A.6
   （`docs/design.md` §9.1 的表已是六项）。
3. **`compose` 语义不对称**：CLI 只打印，MCP/REST 会装载（§6.3）。
4. **`SEED_PRODUCE_OVERLAP` 是类型级判定**，比 §1.4.3 参与者级更宽（假阳性方向，不漏报）；
   作为规范的安全超集登记于设计 D21，不回灌论文母本。
5. **Rule-R 不入事件日志**（D9）。可选 `read_audit = true` 将读审计写入独立审计流（§7.5，fail-closed）；未开启时需在外层自行记录。
6. **Archival Policy 未内置**（§3.2），需要时用一个持 `consumes` gate 的参与者表达。
7. **无 `daemon stop`**；systemd unit 与容器镜像由阶段 9 交付（`dist/pactd.service`、
   `dist/Dockerfile`，§2.5）；**PID 文件已提供**（`pid_file` 可选配置，含陈旧检测），
   停机靠 SIGINT/SIGTERM 优雅退出（§2.3）。
8. **传输与身份范围**：NFR6 v2 已实现 TLS/mTLS、Ed25519 签名、MCP Streamable HTTP 与 MCP OIDC 资源服务器（D34–D36）。仍有边界：daemon 不内置授权服务器/授权码 + PKCE；OIDC 仅 `/mcp`；内置 CLI/MCP/runtime 不出示客户端证书；`pact-runtime` 的 stdio MCP 子进程不透传签名参数；OIDC 令牌不缓存、不转发 REST（§2.6，架构 §4）。
9. **旧日志不自动迁移**（D27）：双键布局只识别 `<data_dir>/<domain>/<instance>.events.jsonl`；顶层遗留单层文件启动即 `LegacyLayoutDetected` fail-fast，须人工迁移（§7.1），不静默丢数据。
10. **跨实例不协同**（D27）：同一域声明的多个 Counter 实例彼此隔离（各自 Counter/Archive/订阅）；跨实例协同不在范式保障范围内（论文 rev3 §1.3 实现自由条款，见 §1.3）。
11. **压缩会牺牲前缀审计**（D25）：被丢弃前缀不再可查，审计视图 = 自足快照 + 保留尾部事件；不做自动周期性轮转，压缩为运维显式动作或启动开关（§7.4）。
12. **CPN 编码的边界**（D28，§7.7）：负条件依赖调用方给出的 `--bound` 真实并发上界（不可从声明推导）；`not via` 不支持；CPN Tools GUI 内的实际加载/状态空间实验需人工验收，CI 只保证 XML/DTD 与结构断言；TokenId 生成器使用 CPN ML `ref` 副作用，状态空间工具可能将其视为非纯表达式。
13. **MCP `/mcp` 请求体上限**：rmcp 的 Streamable HTTP 默认请求体上限独立于签名 8 MiB 限制；超限由 MCP 层拒绝（非签名错误）。
14. **DSH 插件已完成真实装配与真实模型门控验证**（D37，阶段 11）：插件包 `dsh-plugin-pact`（独立仓库 `onlookor/pact-dsh`）通过 Node 内置测试验证配置、治理与路由逻辑（47 项），已在 `@deepseek-ai/dsh@0.2.1-rc.2` 上完成 profile 安装、`--dump-config` 组合与启动验证（运行时 `ctx.agentLoop` 为 `PactAgentLoop`），并在 `deepseek-official/deepseek-flash` 会话中验证允许、`denyTools` 拒绝、`fakeEnabled=false` 预检拒绝三条路径。WASM 后端已 live（`wasm-bindgen 0.2.127`）；`resume` 的首步不受门控（§13）。

---

## 12. 上手路径

1. 读 `examples/README.md` §1–§2，跑两条脚本：

   ```bash
   bash examples/offline/run.sh      # 离线三件套：analyze / compose / replay
   bash examples/online/run.sh       # 在线：daemon 全生命周期 15 步（Rule-T 在第 11 步）
   ```

   两者都会把输出录进各自的 `trace.txt`，可与仓库内的录制结果逐行对照。
2. 挑一份与你场景最像的通用域改起：
   `se_lifecycle`（SE 生命周期）、`re_pipeline`（RE 流水线 + 变更回路）、
   `react_loop`（agent 循环）、`cross_harness`（跨工具编排）。
3. 用 §1.4/§1.5 的判据与边界复核选型，用 §5 的六项分析自检。
4. 需要行业/方法论特定的映射（如某个技能体系的产物清单与验收清单）时，
   看 `docs/skill-adaptation.md`——那是本手册之外的**领域适配层**，不属于范式本身。

### 参考

| 文档 | 内容 |
|------|------|
| [`paper/pactv1_rev3.md`](../paper/pactv1_rev3.md) | 范式规范母本：六原语、六规则、§2 适用性、§3 文法、Appendix A 分析算法 |
| [`docs/requirements.md`](requirements.md) | 需求与范围（含明确的范围外项） |
| [`docs/design.md`](design.md) | 实现设计：D1–D37 决策记录、§6 线上格式、§9 接口与身份（TLS/签名/MCP HTTP/OIDC）、§17 DSH 插件 |
| [`docs/architecture.md`](architecture.md) | 架构视图：进程与部署、关键数据流、启动恢复、§7 DSH 宿主集成 |
| [`docs/skill-adaptation.md`](skill-adaptation.md) | 领域适配层示例（RE / SE 技能），与本手册正交 |
| [`examples/README.md`](../examples/README.md) | 五份通用域 + 两条可复现轨迹的逐例讲解 |
| [`dsh-plugin-pact`](https://github.com/onlookor/pact-dsh#readme) | DSH 插件安装、配置与限制（独立仓库） |

## 13. DSH 插件（阶段 11，D37）

`dsh-plugin-pact`（独立仓库 `onlookor/pact-dsh`）是 DeepSeek Harness（DSH）的 PACT 治理插件。它**替换 agent loop 的工厂注册者**，但机械的 turn/step/工具执行仍由 DSH 官方 `ReactLoopAgent` 完成；PACT 只决定“这一步能不能开始、这个工具能不能执行”。

### 13.1 能力与档位

| 档位 | 行为 | 是否阻断 |
|---|---|---|
| `observe`（缺省） | 把 `session/event` 裁剪为 `type/seq/工具名/call id` 写入有界缓冲；配置 `mirrorDir` 时追加 JSONL | 否 |
| `govern` | 在 `agent/pre-step` 与 `tools/pre-execute` 查询 PACT 门；未启用返回 `reject` / `deny` | 是 |

只有 `agentPreset` 以 `pact-` 开头的 agent 进入治理；其他 agent（含 DSH 默认 preset）完全旁路，行为与官方 loop 一致。`tools/post-execute` 与 `agent/turn-stopping` 只做审计；`stopOnTermination = true` 时 Rule-T 终止后会 steer 收尾。

### 13.2 装载

1. 把 `dsh-plugin-pact` 作为 bundle 加入 DSH 的 profile：`dsh plugin --profile <name> add github:onlookor/pact-dsh`（或 `link:<本地克隆路径>`）。插件通过 `cordis.patch.yml` **禁用**原 `agent-loop` 行，并插入 `pact-agent-loop` 与观察行；DSH 的 patch 不能改写行的 `name`。
2. 包内零运行时依赖；`@deepseek-ai/dsh-agent-loop` 是 peer，由 DSH 安装解析。
3. 若使用 WASM 后端，先构建产物（与 CI 同一条命令，`wasm-bindgen` 版本须为 `0.2.127`）：

```bash
cargo build --locked -p pact-wasm --target wasm32-unknown-unknown --profile wasm-release
wasm-bindgen --target nodejs --out-dir /tmp/pact-wasm-pkg \
  target/wasm32-unknown-unknown/wasm-release/pact_wasm.wasm
```

   再把 `wasmModule` 指向仓库内的组装入口 `crates/pact-wasm/js/open-host.js`，由它把产物、`FileStore` 与声明组装成 `PactHost`；产物或参数缺失时治理调用返回 `BackendUnavailable`（fail-closed 拒绝，不崩溃）。

已在 `@deepseek-ai/dsh@0.2.1-rc.2` 上验证的真实装载步骤：

```bash
npm install --prefix ~/.local pnpm@11.28.5
export PATH="$HOME/.local/node_modules/.bin:$PATH"
export DSH_HOME=/tmp/dsh-home
npx -y @deepseek-ai/dsh@0.2.1-rc.2 --profile pact-web --from-default-profile web --dump-config
npx -y @deepseek-ai/dsh@0.2.1-rc.2 plugin --profile pact-web add github:onlookor/pact-dsh
npx -y @deepseek-ai/dsh@0.2.1-rc.2 --profile pact-web --dump-config   # 确认 disabled + pact-agent-loop
npx -y @deepseek-ai/dsh@0.2.1-rc.2 --profile pact-web --no-open       # 启动
```

注意：禁用原行会丢弃其 config（当前 base 为 `agents: []`）；若 profile 配置了 `agents`，请写进插件的 `pact-agent-loop` 行。

### 13.3 配置

配置只从环境变量 `PACT_DSH_CONFIG` 读取：值可以是 JSON 文件路径，也可以直接是内联 JSON。缺省为 `observe` + `fake` 后端。

```json
{
  "mode": "govern",
  "backend": "fake",
  "domain": "dsh",
  "defaultParticipant": "agent",
  "presetPrefix": "pact-",
  "participants": { "pact-review": "reviewer" },
  "mirrorDir": "/home/me/.pact/dsh-audit",
  "mirrorLimit": 256,
  "denyTools": ["shell"],
  "stopOnTermination": false,
  "fakeEnabled": true
}
```

- `mode=govern` 时 fail-closed 强制开启：后端不可用即拒绝，不会放行。
- `backend=wasm` 时必须提供 `wasmModule`（模块路径或包名），并通常配合 `wasmOptions`：

```json
{
  "backend": "wasm",
  "wasmModule": "/home/me/pact/crates/pact-wasm/js/open-host.js",
  "wasmOptions": {
    "module": "/tmp/pact-wasm-pkg",
    "storeRoot": "/home/me/.pact/dsh-wasm",
    "declaration": "types { … } rules { … }",
    "ts": "2026-10-23T00:00:00.000Z"
  }
}
```

  `wasmOptions` 原样透传给 `openPactHost`：`module` 为 wasm-bindgen out-dir（或 `pact_wasm.js`），`storeRoot` 为事件日志根目录（布局与原生 daemon 一致），`declaration`/`ts` 仅在首次新建实例时使用，已有日志则忽略。插件只校验 `wasmOptions` 是对象，具体键由宿主模块校验。
- `fakeEnabled=false` 让假后端对所有参与者返回未启用，用于预演 `agent/pre-step` 拒绝；仅对 `backend=fake` 生效。
- 配置非法（未知 mode/backend、非法域名或参与者名、mirrorLimit 越界、坏 JSON）会在插件加载期直接报错。

### 13.4 审计与隐私

- 只记录事件类型、序号、工具名与调用 id；**不记录**工具参数、结果、源码或用户内容。
- `mirrorDir` 目录 `0700`、文件 `0600`；不新增监听端口、不拉起子进程。
- 实例键由 session id 消毒（非 `[A-Za-z0-9_-]` 替换为 `_`，上限 128 字符），避免路径穿越。

### 13.5 已知限制

- `resume` 的钩子在会话发布后安装，**恢复后的第一步不受门控**。
- 本轮不做 token 级工具映射（工具成功/失败对应 produce/consume 与补偿）、不注册 PACT 原生工具。
- 依赖 DSH 预发布 API；DSH 升级后需复核 `AgentLoop` 继承与钩子签名。
- 真实装配已验证到“插件加载 + `agentLoop` 服务替换”；并在真实 `deepseek-official/deepseek-flash` 会话中验证了 `pact-*` 门控三条路径（允许 / `denyTools` 拒绝 / `fakeEnabled=false` 预检拒绝）。
- `backend=wasm` 已在 headless profile 中用 `wasm-bindgen 0.2.127` 产物复跑同一三条路径（允许时 bash 实际执行、`denyTools` 拒绝、空 Counter 实例的 `pre-step` 拒绝），WASM 实例日志由插件运行时创建。验证用模型为临时 mock provider，不随仓库分发。
- 插件进程退出时不显式关闭 WASM 宿主，实例锁文件可能残留；下次打开按 pid 存活检测自动回收（同一实例跨进程复跑已验证）。
