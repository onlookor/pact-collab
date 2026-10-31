'use strict';

// P4/S2 测试：Node 存储适配器 + WASM 内核。
// 前置：wasm-bindgen 产物存在（`PACT_WASM_PKG` 指向 nodejs 目标输出目录）。
// 运行：PACT_WASM_PKG=/tmp/pact-wasm-pkg node --test crates/pact-wasm/js/test

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { FileStore } = require('../pact-store');
const { PactHost } = require('../pact-host');

const PKG = process.env.PACT_WASM_PKG || path.join(__dirname, '..', 'pkg');
const PKG_ENTRY =
  fs.existsSync(PKG) && fs.statSync(PKG).isDirectory() ? path.join(PKG, 'pact_wasm.js') : PKG;
let PactWasm;
try {
  ({ PactWasm } = require(PKG_ENTRY));
} catch (e) {
  throw new Error(
    `无法加载 wasm 包：${PKG}（先构建 wasm32 产物并用 PACT_WASM_PKG 指向 bindgen 输出）`,
  );
}

const DECL = `types { requirement; design }
preconditions { requirement → design }
gates {
    architect: reads requirement, consumes requirement, produces design;
    user: consumes design
}
seed_authority { external: seeds requirement }
`;

function tmpRoot() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'pact-js-'));
}

function open(root, refBindings) {
  return PactHost.open({
    createWasm: () => new PactWasm(),
    store: new FileStore(root),
    domain: 'pipeline',
    instance: 'default',
    declaration: DECL,
    ts: 'T0',
    refBindings,
  });
}

function eventsFile(root) {
  return path.join(root, 'pipeline', 'default.events.jsonl');
}

function seedAndProduce(host) {
  const req = host.seed('external', 'requirement', 'urn:req:1', 'seq-0', 'T1');
  const design = host.produce('architect', 'design', [req.id], 'urn:design:1', 'seq-1', 'T2');
  return { req, design };
}

test('生命周期落盘后重启状态一致', () => {
  const root = tmpRoot();
  let host = open(root);
  const { req, design } = seedAndProduce(host);
  const entry = host.consume('architect', req.id, 'rid-1', 'T3');
  assert.equal(entry.token.id, 'seq-0');
  assert.equal(host.status().counter_size, 1);
  assert.equal(host.archive().size, 1);
  assert.equal(host.exportEvents().trim().split('\n').length, 4);
  host.close();

  host = open(root);
  assert.equal(host.status().counter_size, 1);
  assert.equal(host.archive().size, 1);
  assert.equal(host.counter().tokens[0].id, design.id);
  assert.equal(host.archive().entries[0].token.id, req.id);
  host.close();
});

test('append 后未 commit 即崩溃：日志优先，重启可见该事件', () => {
  const root = tmpRoot();
  const host = open(root);
  // 直接写 prepare 返回的行，模拟「宿主 append+fsync 后进程崩溃、WASM 未 commit」。
  const prep = JSON.parse(
    host.wasm.prepare_seed('pipeline/default', 'external', 'requirement', null, 'seq-0', 'T1'),
  );
  new FileStore(root).append('pipeline', 'default', prep.event);
  host.close();

  const reopened = open(root);
  assert.equal(reopened.status().counter_size, 1);
  assert.equal(reopened.counter().tokens[0].id, 'seq-0');
  reopened.close();
});

test('末尾半行被截断，先前完整事件不受影响', () => {
  const root = tmpRoot();
  let host = open(root);
  host.seed('external', 'requirement', null, 'seq-0', 'T1');
  host.close();

  fs.appendFileSync(eventsFile(root), '{"seq":9,"marker":"half-written');
  host = open(root);
  assert.equal(host.status().counter_size, 1);
  const text = fs.readFileSync(eventsFile(root), 'utf8');
  assert.ok(text.endsWith('\n'), '清洗后日志以换行结束');
  assert.ok(!text.includes('marker'), '半行已被截断');
  host.close();
});

test('单写者约束：同实例二次打开被拒绝，释放后可重开', () => {
  const root = tmpRoot();
  const first = open(root);
  assert.throws(
    () => open(root),
    (err) => err.code === 'InstanceLocked',
  );
  first.close();
  const second = open(root);
  second.close();
});

test('快照自足：日志清空后仍可从快照恢复', () => {
  const root = tmpRoot();
  let host = open(root);
  seedAndProduce(host);
  const snap = host.snapshot();
  assert.equal(snap.seq, 3);
  assert.equal(Object.keys(snap.state.counter).length, 2);
  host.close();

  fs.writeFileSync(eventsFile(root), '');
  host = open(root);
  assert.equal(host.status().counter_size, 2);
  assert.equal(host.counter().tokens.length, 2);
  host.close();
});

test('幂等键跨重启生效：重放不写新事件', () => {
  const root = tmpRoot();
  let host = open(root);
  const { req } = seedAndProduce(host);
  host.consume('architect', req.id, 'rid-1', 'T3');
  const before = host.exportEvents();
  host.close();

  host = open(root);
  const replay = host.consume('architect', req.id, 'rid-1', 'T4');
  assert.equal(replay.token.id, req.id);
  assert.equal(host.exportEvents(), before, '幂等重放不应改变日志');
  host.close();
});

test('refBindings 注入后在写入前校验并规范化票面', () => {
  const root = tmpRoot();
  const refBindings = {
    apiVersion: 'pact.ref/v1',
    kind: 'RefBindings',
    metadata: { domain: 'pipeline' },
    spec: {
      types: {
        requirement: { ref: { required: true, schemes: ['https'] } },
      },
    },
  };
  let host = open(root, refBindings);

  assert.throws(
    () => host.seed('external', 'requirement', null, 'seq-0', 'T1'),
    (e) => e.code === 'ThingRefRequired',
    '省略/null 在 required 规则下应被拒绝',
  );
  assert.throws(
    () => host.seed('external', 'requirement', 'git://example.com/req', 'seq-0', 'T1'),
    (e) => e.code === 'InvalidThingRef',
    '白名单外的 scheme 应被拒绝',
  );

  host.seed('external', 'requirement', 'HTTPS://example.com/req', 'seq-0', 'T1');
  assert.equal(
    host.counter().tokens[0].thing_ref,
    'https://example.com/req',
    'scheme 应规范化为小写',
  );

  // 运行期移除绑定后回到结构校验。
  assert.equal(host.removeRefBindings(), true);
  host.seed('external', 'requirement', 'REQ-001', 'seq-1', 'T2');
  assert.equal(host.counter().tokens.length, 2);
  host.close();
});

test('Rule-T 终止事件落盘，重启后终止态一致', () => {
  const root = tmpRoot();
  let host = open(root);
  const { req, design } = seedAndProduce(host);
  host.consume('architect', req.id, 'rid-1', 'T3');
  host.consume('user', design.id, 'rid-2', 'T4');
  assert.equal(host.status().terminated, true);
  const log = host.exportEvents();
  assert.ok(log.includes('"kind":"Terminated"'), '终止事件应写入日志');
  host.close();

  host = open(root);
  assert.equal(host.status().terminated, true);
  assert.equal(host.status().counter_size, 0);
  host.close();
});
