'use strict';

// 组装入口：wasm-bindgen 产物 + Node 存储 → PactHost。
//
// DSH 插件以 `backend=wasm` 运行时会动态 import 本模块并调用
// `openPactHost({ domain, instance, ...wasmOptions })`；模块必须是 CommonJS
// 可导入形态（DSH 侧同时兼容 `default.openPactHost`）。
//
// 选项（缺省值可由环境变量提供，便于宿主进程零配置注入）：
//   module       wasm-bindgen out-dir 或其 `pact_wasm.js`（PACT_WASM_MODULE）
//   storeRoot    事件日志根目录，布局与原生 daemon 一致（PACT_WASM_STORE）
//   domain       域名（必填）
//   instance     实例名（必填）
//   declaration  仅新建实例时使用（PACT_WASM_DECLARATION）
//   ts           新建实例的 DomainLoaded 时间戳（PACT_WASM_TS，缺省当前时间）
//   refBindings  RefBindings 对象/数组/JSON 字符串，宿主注入给内核
//                （PACT_WASM_REF_BINDINGS，JSON 字符串）；缺省只做结构校验
//
// 已有的 `<storeRoot>/<domain>/<instance>.events.jsonl` 存在时以日志为准，
// `declaration`/`ts` 被忽略（与 `PactHost.open` 的恢复路径一致）。

const fs = require('node:fs');
const path = require('node:path');
const { PactHost } = require('./pact-host');
const { FileStore } = require('./pact-store');

function requireString(value, field) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new TypeError(`openPactHost: ${field} must be a non-empty string`);
  }
  return value;
}

function resolveGlue(modulePath) {
  const resolved = path.resolve(modulePath);
  if (fs.existsSync(resolved) && fs.statSync(resolved).isDirectory()) {
    return path.join(resolved, 'pact_wasm.js');
  }
  return resolved;
}

function openPactHost(options = {}) {
  const modulePath = requireString(
    options.module ?? process.env.PACT_WASM_MODULE,
    'module',
  );
  const storeRoot = requireString(
    options.storeRoot ?? process.env.PACT_WASM_STORE,
    'storeRoot',
  );
  const domain = requireString(options.domain, 'domain');
  const instance = requireString(options.instance, 'instance');
  const declaration = options.declaration ?? process.env.PACT_WASM_DECLARATION ?? '';
  if (typeof declaration !== 'string') {
    throw new TypeError('openPactHost: declaration must be a string');
  }
  const ts = options.ts ?? process.env.PACT_WASM_TS ?? new Date().toISOString();
  const refBindings =
    options.refBindings ?? process.env.PACT_WASM_REF_BINDINGS ?? null;

  const { PactWasm } = require(resolveGlue(modulePath));
  const store = new FileStore(storeRoot);
  return PactHost.open({
    createWasm: () => new PactWasm(),
    store,
    domain,
    instance,
    declaration,
    ts,
    refBindings,
  });
}

module.exports = { openPactHost };
