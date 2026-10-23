'use strict';

// Q7 CI 冒烟与门禁：加载 wasm-bindgen 产物、校验 ABI、跑最小语义路径、断言体积上限。
//
// 用法（CI）：
//   PACT_WASM_PKG=<bindgen out-dir> [PACT_WASM_WASM=<path/to/pact_wasm_bg.wasm>] node crates/pact-wasm/js/ci-smoke.js
//
// 体积上限刻意留出约 30% 余量（基线 450,190 B / gzip 140,457 B），只拦截显著膨胀。

const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');

const PKG = process.env.PACT_WASM_PKG || path.join(__dirname, 'pkg');
const WASM = process.env.PACT_WASM_WASM || path.join(PKG, 'pact_wasm_bg.wasm');
const EXPECTED_ABI = 'pact-wasm/0.2';
const MAX_RAW_BYTES = 600 * 1024;
const MAX_GZIP_BYTES = 200 * 1024;

const ENTRY =
  fs.existsSync(PKG) && fs.statSync(PKG).isDirectory() ? path.join(PKG, 'pact_wasm.js') : PKG;
const { PactWasm } = require(ENTRY);

const DECL = `types { requirement; design }
preconditions { requirement → design }
gates {
    architect: reads requirement, consumes requirement, produces design;
    user: consumes design
}
seed_authority { external: seeds requirement }
`;

function assert(cond, message) {
  if (!cond) {
    console.error(`ci-smoke 失败：${message}`);
    process.exit(1);
  }
}

const wasm = new PactWasm();

const abi = wasm.abi_version();
assert(abi === EXPECTED_ABI, `ABI 期望 ${EXPECTED_ABI}，实际 ${abi}`);

const validation = JSON.parse(wasm.validate(DECL));
assert(validation.types.length === 2, `类型数期望 2，实际 ${validation.types.length}`);
assert(validation.gates === 4, `gates 子句数期望 4，实际 ${validation.gates}`);
assert(validation.seed_authority === 1, `seed_authority 期望 1，实际 ${validation.seed_authority}`);

const report = JSON.parse(wasm.analyze(DECL));
assert(report.precondition_wf.well_formed === true, 'A.5 良构校验未通过');
assert(
  report.reachability.seed_types.includes('requirement'),
  `源类型期望包含 requirement，实际 ${JSON.stringify(report.reachability.seed_types)}`,
);

assert(fs.existsSync(WASM), `找不到 wasm 产物：${WASM}`);
const raw = fs.readFileSync(WASM);
const gzip = zlib.gzipSync(raw, { level: 9 });
assert(raw.length <= MAX_RAW_BYTES, `raw 体积 ${raw.length} B 超过上限 ${MAX_RAW_BYTES} B`);
assert(gzip.length <= MAX_GZIP_BYTES, `gzip 体积 ${gzip.length} B 超过上限 ${MAX_GZIP_BYTES} B`);

console.log(
  `ci-smoke 通过：ABI ${abi}，raw ${raw.length} B，gzip ${gzip.length} B`,
);

// openPactHost 组装层（产物 + FileStore）的持久化往返：新建 → seed → 关闭 → 重开恢复。
const os = require('node:os');
const { openPactHost } = require('./open-host');

const storeRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'pact-wasm-store-'));
const hostOptions = {
  module: PKG,
  storeRoot,
  domain: 'ci',
  instance: 'smoke',
  declaration: DECL,
  ts: '2026-01-01T00:00:00.000Z',
};

const first = openPactHost(hostOptions);
assert(first.enabled('architect').length === 0, '新实例不应有可生产项');
first.seed('external', 'requirement', undefined, 'req-1', '2026-01-01T00:00:01.000Z');
assert(first.enabled('architect').length > 0, 'seed 后 architect 应可生产设计');
first.close();

// 日志已存在时以日志为准：即便声明非法也不应影响恢复。
const second = openPactHost({ ...hostOptions, declaration: 'types { bogus }' });
assert(second.status().counter_size === 1, '恢复后 Counter 应为 1');
assert(second.enabled('architect').length > 0, '恢复后 architect 应仍可生产');
second.close();
fs.rmSync(storeRoot, { recursive: true, force: true });
console.log('openPactHost 持久化往返通过');
