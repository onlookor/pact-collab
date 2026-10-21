'use strict';

// P4/S2：Node 版 PACT 存储适配器。
//
// 真相源 = `<root>/<domain>/<instance>.events.jsonl`（append + fsync）；
// 快照 = `<root>/<domain>/<instance>.snapshot.json`（tmp + rename + 目录 fsync）；
// 单写者 = `<root>/<domain>/<instance>.lock`（O_EXCL 创建，写入 pid，关闭时删除）。
//
// 与原生布局逐字一致，因此原生 daemon 与 WASM 宿主可交替打开同一日志。
// 本模块不做语义判断；所有规则由 WASM 内核（pact-wasm）执行。

const fs = require('node:fs');
const path = require('node:path');

const KEY_RE = /^[A-Za-z0-9_-]{1,128}$/;

class StoreError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'StoreError';
    this.code = code;
  }
}

function checkKey(domain, instance) {
  if (!KEY_RE.test(domain)) {
    throw new StoreError('InvalidDomainName', `非法域名：${domain}`);
  }
  if (!KEY_RE.test(instance)) {
    throw new StoreError('InvalidInstanceName', `非法实例名：${instance}`);
  }
}

class FileStore {
  constructor(root) {
    this.root = path.resolve(root);
    this._lockFd = null;
    this._lockPath = null;
  }

  dirOf(domain) {
    if (!KEY_RE.test(domain)) {
      throw new StoreError('InvalidDomainName', `非法域名：${domain}`);
    }
    return path.join(this.root, domain);
  }

  eventsPath(domain, instance) {
    checkKey(domain, instance);
    return path.join(this.root, domain, `${instance}.events.jsonl`);
  }

  snapshotPath(domain, instance) {
    checkKey(domain, instance);
    return path.join(this.root, domain, `${instance}.snapshot.json`);
  }

  lockPath(domain, instance) {
    checkKey(domain, instance);
    return path.join(this.root, domain, `${instance}.lock`);
  }

  ensureDir(domain) {
    fs.mkdirSync(this.dirOf(domain), { recursive: true });
  }

  /** 获取实例独占锁；同实例二次打开抛 `InstanceLocked`。 */
  acquire(domain, instance) {
    this.ensureDir(domain);
    const lock = this.lockPath(domain, instance);
    try {
      const fd = fs.openSync(lock, 'wx');
      fs.writeSync(fd, String(process.pid));
      fs.fsyncSync(fd);
      this._lockFd = fd;
      this._lockPath = lock;
      return;
    } catch (e) {
      if (e.code !== 'EEXIST') throw e;
    }
    // 陈旧锁：持有者进程已不存在时回收一次。
    const owner = Number.parseInt(fs.readFileSync(lock, 'utf8'), 10);
    if (Number.isInteger(owner) && owner > 0 && !isAlive(owner)) {
      fs.unlinkSync(lock);
      const fd = fs.openSync(lock, 'wx');
      fs.writeSync(fd, String(process.pid));
      fs.fsyncSync(fd);
      this._lockFd = fd;
      this._lockPath = lock;
      return;
    }
    throw new StoreError(
      'InstanceLocked',
      `实例 ${domain}/${instance} 已被 pid ${owner || '?'} 占用（单写者约束）`,
    );
  }

  release() {
    if (this._lockFd !== null) {
      try {
        fs.closeSync(this._lockFd);
      } finally {
        this._lockFd = null;
      }
    }
    if (this._lockPath) {
      try {
        fs.unlinkSync(this._lockPath);
      } catch (e) {
        if (e.code !== 'ENOENT') throw e;
      }
      this._lockPath = null;
    }
  }

  /** 追加一行并 fsync；失败时抛出，调用方负责 abort 事务。 */
  append(domain, instance, line) {
    this.ensureDir(domain);
    const fd = fs.openSync(this.eventsPath(domain, instance), 'a');
    try {
      fs.writeSync(fd, line.endsWith('\n') ? line : `${line}\n`);
      fs.fsyncSync(fd);
    } finally {
      fs.closeSync(fd);
    }
  }

  /**
   * 读取并清洗日志：仅容忍末尾半行（进程崩溃留下的截断行），
   * 按字节截断到最后一条完整换行；中部非法行交由 WASM `restore` fail-fast。
   */
  readEvents(domain, instance) {
    const file = this.eventsPath(domain, instance);
    if (!fs.existsSync(file)) return [];
    const buf = fs.readFileSync(file);
    if (buf.length === 0) return [];
    const last = buf.lastIndexOf(0x0a);
    if (last !== buf.length - 1) {
      fs.truncateSync(file, last + 1);
      const fd = fs.openSync(file, 'r+');
      try {
        fs.fsyncSync(fd);
      } finally {
        fs.closeSync(fd);
      }
      if (last < 0) return [];
      return buf.subarray(0, last + 1).toString('utf8').split('\n').filter(Boolean);
    }
    return buf.toString('utf8').split('\n').filter(Boolean);
  }

  readSnapshot(domain, instance) {
    const file = this.snapshotPath(domain, instance);
    if (!fs.existsSync(file)) return null;
    return fs.readFileSync(file, 'utf8');
  }

  /** 原子写快照：tmp 写入 + fsync + rename + 目录 fsync。 */
  writeSnapshot(domain, instance, json) {
    this.ensureDir(domain);
    const target = this.snapshotPath(domain, instance);
    const tmp = `${target}.tmp-${process.pid}`;
    const fd = fs.openSync(tmp, 'w');
    try {
      fs.writeSync(fd, json);
      fs.fsyncSync(fd);
    } finally {
      fs.closeSync(fd);
    }
    fs.renameSync(tmp, target);
    const dirFd = fs.openSync(this.dirOf(domain), 'r');
    try {
      fs.fsyncSync(dirFd);
    } finally {
      fs.closeSync(dirFd);
    }
  }
}

function isAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === 'EPERM';
  }
}

module.exports = { FileStore, StoreError };
