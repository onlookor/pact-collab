'use strict';

// P4/S2：WASM 内核 + Node 存储的最小宿主。
//
// 写路径遵循「日志先行」：
//   prepare（分配 seq，不改内存状态）→ append + fsync → commit（应用状态）
//   append 失败则 abort，内存态与日志均不前进。
// 打开路径：有日志则以日志为准（可带快照），无日志则新建实例。

const { StoreError } = require('./pact-store');

function toStoreError(e) {
  const text = String(e);
  try {
    const parsed = JSON.parse(text);
    return new StoreError(parsed.code || 'Internal', parsed.message || text);
  } catch {
    return new StoreError('Internal', text);
  }
}

function invoke(fn) {
  let raw;
  try {
    raw = fn();
  } catch (e) {
    throw toStoreError(e);
  }
  return JSON.parse(raw);
}

/** 调用无返回值 / 非 JSON 返回值的 wasm 方法（错误仍统一解包）。 */
function call(fn) {
  try {
    return fn();
  } catch (e) {
    throw toStoreError(e);
  }
}

/**
 * 把 `refBindings`（对象、对象数组或 JSON 字符串）注入 wasm 内核。
 *
 * 绑定由宿主提供，WASM 不读文件；重复注入同域绑定即替换。
 */
function applyRefBindings(wasm, refBindings) {
  if (refBindings === undefined || refBindings === null) return;
  let entries;
  if (typeof refBindings === 'string') {
    entries = [refBindings];
  } else if (Array.isArray(refBindings)) {
    entries = refBindings.map((item) =>
      typeof item === 'string' ? item : JSON.stringify(item),
    );
  } else if (typeof refBindings === 'object') {
    entries = [JSON.stringify(refBindings)];
  } else {
    throw new TypeError(
      'refBindings must be an object, an array of objects, or a JSON string',
    );
  }
  for (const json of entries) {
    call(() => wasm.set_ref_bindings(json));
  }
}

class PactHost {
  constructor(wasm, store, domain, instance) {
    this.wasm = wasm;
    this.store = store;
    this.domain = domain;
    this.instance = instance;
    this.key = `${domain}/${instance}`;
    this.closed = false;
  }

  /**
   * @param {object} opts
   * @param {() => object} opts.createWasm 返回新建 PactWasm 实例
   * @param {import('./pact-store').FileStore} opts.store
   * @param {string} opts.domain
   * @param {string} opts.instance
   * @param {string} opts.declaration 新建时使用；已有日志时以日志内声明为准
   * @param {string} opts.ts 新建实例的 DomainLoaded 时间戳
   * @param {object|object[]|string} [opts.refBindings] RefBindings（对象/数组/JSON 字符串）
   */
  static open(opts) {
    const { createWasm, store, domain, instance, declaration, ts, refBindings } = opts;
    store.acquire(domain, instance);
    const host = new PactHost(createWasm(), store, domain, instance);
    try {
      // 绑定先于实例加载/恢复注入；写入路径随即受其约束。
      applyRefBindings(host.wasm, refBindings);
      host._initialize(declaration, ts);
      return host;
    } catch (e) {
      host.close();
      throw e;
    }
  }

  /** 运行期注入/替换绑定（对象、对象数组或 JSON 字符串）。 */
  setRefBindings(refBindings) {
    applyRefBindings(this.wasm, refBindings);
  }

  /** 移除一个域的绑定；返回是否确有移除。 */
  removeRefBindings(domain = this.domain) {
    return call(() => this.wasm.remove_ref_bindings(domain));
  }

  /** 清空全部绑定，回到只做结构校验。 */
  clearRefBindings() {
    call(() => this.wasm.clear_ref_bindings());
  }

  _initialize(declaration, ts) {
    const lines = this.store.readEvents(this.domain, this.instance);
    const snapshot = this.store.readSnapshot(this.domain, this.instance);
    // 空日志且无快照 ⇒ 全新实例；否则一律以日志/快照为准（含压缩后仅剩快照的情形）。
    if (lines.length === 0 && snapshot === null) {
      invoke(() =>
        this.wasm.load_domain_at(this.domain, declaration, this.instance, ts),
      );
      const created = this.wasm.export_events(this.key);
      this.store.append(this.domain, this.instance, created);
      return;
    }
    invoke(() =>
      this.wasm.restore(this.domain, this.instance, lines.join('\n'), snapshot),
    );
  }

  _finish(prep, terminatedTs) {
    if (prep.tx_id === null) return prep.result; // 幂等命中：无事件
    try {
      this.store.append(this.domain, this.instance, prep.event);
    } catch (e) {
      this.wasm.abort(prep.tx_id);
      throw e;
    }
    const committed = invoke(() => this.wasm.commit(prep.tx_id, terminatedTs || null));
    if (committed.terminated) {
      // Terminated 为信息性事件；append 失败不回滚已提交状态（与原生一致）。
      this.store.append(this.domain, this.instance, committed.terminated.event);
    }
    return prep.result;
  }

  seed(participant, type, thingRef, tokenId, ts) {
    const prep = invoke(() =>
      this.wasm.prepare_seed(this.key, participant, type, thingRef, tokenId, ts),
    );
    return this._finish(prep, null);
  }

  produce(participant, type, predecessors, thingRef, tokenId, ts) {
    const prep = invoke(() =>
      this.wasm.prepare_produce(
        this.key,
        participant,
        type,
        JSON.stringify(predecessors),
        thingRef,
        tokenId,
        ts,
      ),
    );
    return this._finish(prep, null);
  }

  consume(participant, tokenId, requestId, ts) {
    const prep = invoke(() =>
      this.wasm.prepare_consume(this.key, participant, tokenId, requestId, ts),
    );
    return this._finish(prep, null);
  }

  read(participant, tokenId) {
    return invoke(() => this.wasm.read(this.key, participant, tokenId));
  }

  counter(filter = null) {
    return invoke(() => this.wasm.counter(this.key, filter));
  }

  archive(filter = null) {
    return invoke(() => this.wasm.archive(this.key, filter));
  }

  gates(participant) {
    return invoke(() => this.wasm.gates(this.key, participant));
  }

  enabled(participant) {
    return invoke(() => this.wasm.enabled(this.key, participant));
  }

  status() {
    return invoke(() => this.wasm.status(this.key));
  }

  /** 导出当前事件日志（JSONL，末行含换行）。 */
  exportEvents() {
    return this.wasm.export_events(this.key);
  }

  /** 原子落盘当前快照。 */
  snapshot() {
    const json = this.wasm.export_snapshot(this.key);
    this.store.writeSnapshot(this.domain, this.instance, json);
    return JSON.parse(json);
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    this.store.release();
  }
}

module.exports = { PactHost, toStoreError };
