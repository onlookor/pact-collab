import test from 'node:test';
import assert from 'node:assert/strict';
import { PactBackendError } from '../src/pact-backend.js';
import { createWasmBackend } from '../src/wasm-backend.js';

test('construction requires a module or a host loader', () => {
  assert.throws(() => createWasmBackend({}), PactBackendError);
});

test('enabled collapses Rule-W production options into a boolean', async () => {
  const options = [
    { type: 'thought', witness_predecessors: ['obs-1'] },
  ];
  const backend = createWasmBackend({ loadHost: async () => ({ enabled: async () => options }) });
  assert.equal(await backend.enabled('agent'), true);

  const idle = createWasmBackend({ loadHost: async () => ({ enabled: async () => [] }) });
  assert.equal(await idle.enabled('agent'), false);
});

test('instance-scoped host drops the key argument for every action', async () => {
  const calls = [];
  const record = (method) => async (...args) => {
    calls.push([method, ...args]);
    return {};
  };
  const backend = createWasmBackend({
    loadHost: async () => ({
      seed: record('seed'),
      produce: record('produce'),
      consume: record('consume'),
      status: record('status'),
    }),
  });

  await backend.seed('dsh/s1', 'agent', 'observation', null, 'tok-1', 'ts');
  await backend.produce('dsh/s1', 'agent', 'thought', [], null, 'tok-2', 'ts');
  await backend.consume('dsh/s1', 'agent', 'tok-1', 'req-1', 'ts');
  await backend.status('dsh/s1');

  assert.deepEqual(calls, [
    ['seed', 'agent', 'observation', null, 'tok-1', 'ts'],
    ['produce', 'agent', 'thought', [], null, 'tok-2', 'ts'],
    ['consume', 'agent', 'tok-1', 'req-1', 'ts'],
    ['status'],
  ]);
});

test('missing host method or failed load surfaces as BackendUnavailable', async () => {
  const missing = createWasmBackend({ loadHost: async () => ({}) });
  await assert.rejects(
    () => missing.enabled('agent'),
    (error) => error instanceof PactBackendError && error.code === 'BackendUnavailable',
  );

  const failing = createWasmBackend({
    loadHost: async () => {
      throw new Error('no wasm artifact');
    },
  });
  await assert.rejects(
    () => failing.status(),
    (error) => error instanceof PactBackendError
      && error.code === 'BackendUnavailable'
      && error.message === 'no wasm artifact',
  );
});

test('close is idempotent and releases an opened host', async () => {
  let closes = 0;
  const backend = createWasmBackend({
    loadHost: async () => ({ status: async () => ({ terminated: false }), close: async () => { closes += 1; } }),
  });

  await backend.close();
  assert.equal(closes, 0, 'a lazily unopened host is not loaded just to close');

  await assert.rejects(
    () => backend.status('dsh/s1'),
    (error) => error instanceof PactBackendError && error.code === 'BackendUnavailable',
  );

  const opened = createWasmBackend({
    loadHost: async () => ({ status: async () => ({ terminated: false }), close: async () => { closes += 1; } }),
  });
  await opened.status('dsh/s1');
  await opened.close();
  await opened.close();
  assert.equal(closes, 1);
});
