import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { FileSink, auditRecord, createObserver } from '../src/observer.js';

test('auditRecord keeps only allow-listed fields', () => {
  const record = auditRecord({
    type: 'tool/call',
    seq: 7,
    tool: 'read_file',
    callId: 'call-1',
    arguments: { secret: 'do-not-store' },
    data: { result: 'do-not-store' },
  });
  assert.deepEqual(record, { type: 'tool/call', seq: 7, tool: 'read_file', callId: 'call-1' });
});

test('auditRecord reads nested DSH data without arguments', () => {
  const record = auditRecord({ type: 'tool/result', data: { name: 'shell', id: 'call-2', args: { token: 'x' } } });
  assert.deepEqual(record, { type: 'tool/result', tool: 'shell', callId: 'call-2' });
});

test('observer keeps a bounded ring buffer', () => {
  const observer = createObserver({ limit: 2 });
  observer.record({ type: 'a', seq: 1 });
  observer.record({ type: 'b', seq: 2 });
  observer.record({ type: 'c', seq: 3 });
  assert.deepEqual(observer.snapshot(), [{ type: 'b', seq: 2 }, { type: 'c', seq: 3 }]);
});

test('observer ignores empty events and sink failures do not escape', () => {
  let calls = 0;
  const observer = createObserver({
    sink: { append: () => { calls += 1; throw new Error('disk full'); } },
    onError: () => { calls += 1; },
  });
  observer.record({ arguments: {} });
  assert.equal(observer.snapshot().length, 0);
  observer.record({ type: 'step/start', seq: 1 });
  assert.equal(observer.snapshot().length, 1);
  assert.equal(calls, 2);
});

test('FileSink appends JSONL with owner-only permissions', () => {
  const dir = mkdtempSync(join(tmpdir(), 'pact-dsh-observer-'));
  try {
    const sink = new FileSink(dir);
    const observer = createObserver({ sink });
    observer.record({ type: 'step/start', seq: 1 });
    observer.record({ type: 'step/end', seq: 2 });
    observer.close();

    const lines = readFileSync(join(dir, 'events.jsonl'), 'utf8').trim().split('\n');
    assert.deepEqual(lines.map((line) => JSON.parse(line)), [
      { type: 'step/start', seq: 1 },
      { type: 'step/end', seq: 2 },
    ]);
    assert.equal(statSync(dir).mode & 0o777, 0o700);
    assert.equal(statSync(join(dir, 'events.jsonl')).mode & 0o777, 0o600);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
