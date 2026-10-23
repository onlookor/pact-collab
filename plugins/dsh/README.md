# @pact/dsh-plugin

DeepSeek Harness（DSH）的 PACT 治理插件。它接管 `AgentFactory` 的注册者，但**不重写** DSH 的 `ReactLoopAgent`：PACT 只决定“这一步能不能开始、这个工具能不能执行”，模型调用、工具执行与会话持久化仍由 DSH 完成。

- 阶段 11，决策 D37 / 需求 FR25；详见 `docs/design.md` §17 与 `docs/manual.md` §13。
- 零 npm 依赖；不进入 Cargo workspace、不进入 `dist/`、不改任何 Rust crate。

## 档位

| 档位 | 行为 | 阻断 |
|---|---|---|
| `observe`（缺省） | 镜像 `session/event` 到有界缓冲；配置 `mirrorDir` 时追加 JSONL | 否 |
| `govern` | `agent/pre-step` 与 `tools/pre-execute` 查询 PACT 门，拒绝返回 `reject` / `deny` | 是 |

只有 `agentPreset` 以 `pact-` 开头的 agent 进入治理；其余 agent 原样委托官方 loop。`tools/post-execute` 与 `agent/turn-stopping` 只审计；`stopOnTermination=true` 时 Rule-T 终止后 steer 收尾。

## 安装

1. 把本目录作为 bundle 加入 DSH 的 profile / workspace；`package.json` 的 `dsh.bundle.patch` 指向 `cordis.patch.yml`。
2. `cordis.patch.yml` **禁用**原始 `agent-loop` 行，并插入 `pact-agent-loop`（同一 `agentLoop` 服务）与 `dsh-pact-observe` 两行。
3. 包内没有运行时 npm 依赖；`@deepseek-ai/dsh-agent-loop` 是 peer，由 DSH 运行时解析。

真实装配（已在 `@deepseek-ai/dsh@0.2.1-rc.2` 上验证）：

```bash
# pnpm 仅在 dsh plugin 管理命令中需要
npm install --prefix ~/.local pnpm@11.28.5
export PATH="$HOME/.local/node_modules/.bin:$PATH"
export DSH_HOME=/tmp/dsh-home

# 用 web 模板创建 profile（或用你已有的 profile）
npx -y @deepseek-ai/dsh@0.2.1-rc.2 --profile pact-web --from-default-profile web --dump-config
npx -y @deepseek-ai/dsh@0.2.1-rc.2 plugin --profile pact-web add /home/wgz/projects/pact/plugins/dsh

# 只看组合结果：应看到 agent-loop 带 disabled: true，并出现 pact-agent-loop
npx -y @deepseek-ai/dsh@0.2.1-rc.2 --profile pact-web --dump-config

# 启动后浏览器 UI 可用；治理需要模型与 PACT 后端
npx -y @deepseek-ai/dsh@0.2.1-rc.2 --profile pact-web --no-open
```

注意：

- DSH 的 patch 只按 `id` 覆盖字段，`name` 仅作一致性校验；**不能**把 `agent-loop` 行直接改名为插件模块，否则该 patch 会被跳过（`patch: name mismatch`）。
- 禁用原行会丢弃其 config。当前 base 模板为 `agents: []`；若你的 profile 在该行配置了 `agents`，请写进本插件的 `pact-agent-loop` 行。
- 链接安装（`link:`）时 pnpm 会提示 peer 不会从目标 `node_modules` 解析；DSH 运行时会用安装副本解析 peer，实测可用。若改用 `file:` 协议可消除该提示。

## 配置

只从 `PACT_DSH_CONFIG` 读取：JSON 文件路径或内联 JSON；缺省 `observe` + `fake`。

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

- `mode=govern` 时 `failClosed` 强制为 `true`。
- `backend=wasm` 时必须提供 `wasmModule`。推荐指向仓库内的组装入口 `crates/pact-wasm/js/open-host.js`，并用 `wasmOptions` 提供宿主参数：

```json
{
  "mode": "govern",
  "backend": "wasm",
  "wasmModule": "/home/me/pact/crates/pact-wasm/js/open-host.js",
  "domain": "dsh",
  "wasmOptions": {
    "module": "/tmp/pact-wasm-pkg",
    "storeRoot": "/home/me/.pact/dsh-wasm",
    "declaration": "types { … } rules { … }",
    "ts": "2026-10-23T00:00:00.000Z"
  }
}
```

  先构建产物：`cargo build --locked -p pact-wasm --target wasm32-unknown-unknown --profile wasm-release`，再 `wasm-bindgen --target nodejs --out-dir /tmp/pact-wasm-pkg target/wasm32-unknown-unknown/wasm-release/pact_wasm.wasm`（CLI 版本须为 `0.2.127`）。`wasmOptions` 原样透传；产物缺失时治理调用返回 `BackendUnavailable`（fail-closed）。
- `fakeEnabled=false` 让假后端对所有参与者返回未启用，用于预演 `agent/pre-step` 拒绝路径；仅对 `backend=fake` 生效。
- 非法配置在加载期 fail-fast（`PactConfigError`）。

## 后端

`PactBackend`：`enabled(participant, key)`、`gates`、`status`、`seed`、`produce`、`consume`、`close`；`key = domain/instance`。

- `FakePactBackend`：单测/演示用，只模拟插件消费的表面（参与者启用、token 存在性、request_id 幂等），不解释门表达式。
- `WasmPactBackend`：惰性动态导入 `openPactHost(options)`，适配 `crates/pact-wasm` 的 `PactHost`；宿主返回的 `enabled` 是 Rule-W 可生产选项数组，适配层收窄为布尔（非空即可执行）。

## 测试

```bash
node --check index.js loop.js src/*.js
node --test test/*.test.js
```

当前 7 个测试文件、47 项通过。测试不需要 DSH，也不需要 WASM 产物；`openPactHost` 的真实产物往返由 `crates/pact-wasm/js/ci-smoke.js` 在 CI 的 wasm job 覆盖。

## 审计与安全

- 只记录 `type/seq/工具名/call id`；不记录工具参数、结果、源码或用户内容。
- `mirrorDir` 目录 `0700`、文件 `0600`；不新增端口、不拉子进程、不联网。
- 实例键由 session id 消毒为 `[A-Za-z0-9_-]{1,128}`。

## 已知限制

- `resume` 的钩子在会话发布后安装，恢复后的第一步不受门控。
- 本轮不做 token 级工具映射（produce/consume 与补偿），也不注册 PACT 原生工具。
- 依赖 DSH 预发布 API（`AgentLoop`、`AgentFactory`、`agent/pre-step`、`tools/pre-execute`）；DSH 升级需复核。
- 已用 `@deepseek-ai/dsh@0.2.1-rc.2` 验证真实装载：`--dump-config` 显示原行禁用与替代行插入，启动后 `ctx.agentLoop` 为 `PactAgentLoop` 且继承官方 `AgentLoop`。
- 已在真实 DSH 会话中验证门控（`deepseek-official/deepseek-flash`）：`pact-*` 预设下 `govern` 允许 `bash` 执行并返回 `PACT_TOOL_OK`、`denyTools` 返回 `pact: tool denied by policy`、`fakeEnabled=false` 使 `agent/pre-step` 拒绝并直接结束 turn（无模型请求）。
- 已用 `backend=wasm`（`wasm-bindgen 0.2.127` 产物 + `open-host.js`）在同一 headless profile 复跑上述三条路径并通过；插件退出时不显式关闭宿主，实例锁文件可能残留，下次打开按 pid 存活检测自动回收。

