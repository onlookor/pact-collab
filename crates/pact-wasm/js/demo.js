'use strict';

// P4/S2 最小宿主 demo：走完 requirement → design 生命周期，快照后重启校验。
//
// 用法：
//   cargo build --target wasm32-unknown-unknown -p pact-wasm --profile wasm-release
//   wasm-bindgen --target nodejs --out-dir /tmp/pact-wasm-pkg target/.../pact_wasm.wasm
//   PACT_WASM_PKG=/tmp/pact-wasm-pkg node crates/pact-wasm/js/demo.js [dataDir]

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const pkg = process.env.PACT_WASM_PKG || path.join(__dirname, 'pkg');
const pkgEntry = fs.existsSync(pkg) && fs.statSync(pkg).isDirectory() ? path.join(pkg, 'pact_wasm.js') : pkg;
const { PactWasm } = require(pkgEntry);
const { FileStore } = require('./pact-store');
const { PactHost } = require('./pact-host');

const DECL = `types { requirement; design }
preconditions { requirement → design }
gates {
    architect: reads requirement, consumes requirement, produces design;
    user: consumes design
}
seed_authority { external: seeds requirement }
`;

const root = process.argv[2] || fs.mkdtempSync(path.join(os.tmpdir(), 'pact-demo-'));
const openHost = () =>
  PactHost.open({
    createWasm: () => new PactWasm(),
    store: new FileStore(root),
    domain: 'demo',
    instance: 'default',
    declaration: DECL,
    ts: 'T0',
  });

let host = openHost();
const req = host.seed('external', 'requirement', 'urn:req:1', 'seq-0', 'T1');
const design = host.produce('architect', 'design', [req.id], 'urn:design:1', 'seq-1', 'T2');
console.log('produced:', design.id);
console.log('status  :', host.status());
host.snapshot();
host.close();

host = openHost();
console.log('restart :', host.status());
console.log('counter :', host.counter());
console.log('archive :', host.archive());
console.log('events  :', host.exportEvents().trim().split('\n').length, 'lines');
host.close();
console.log('data dir:', root);
