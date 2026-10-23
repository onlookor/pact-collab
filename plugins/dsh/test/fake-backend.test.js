import test from 'node:test';
import assert from 'node:assert/strict';
import { FakePactBackend, PactBackendError } from '../src/pact-backend.js';

const KEY = 'dsh/sess-1';

test('enabled reflects participants, the enable switch, and the gate', async () => {
  const backend = new FakePactBackend({ participants: ['agent', 'reviewer'] });
  assert.equal(await backend.enabled('agent', KEY), true);
  assert.equal(await backend.enabled('other', KEY), false);

  backend.setEnabled(false);
  assert.equal(await backend.enabled('agent', KEY), false);

  const gated = new FakePactBackend({ gate: ({ participant }) => participant === 'agent' });
  assert.equal(await gated.enabled('agent', KEY), true);
  assert.equal(await gated.enabled('reviewer', KEY), false);
});

test('produce/seed add tokens and status counts them', async () => {
  const backend = new FakePactBackend();
  await backend.seed(KEY, 'agent', 'Task', 'task-1', 'tok-1', '2026-10-22T00:00:00Z');
  await backend.produce(KEY, 'agent', 'Artifact', ['tok-1'], 'artifact-1', 'tok-2', '2026-10-22T00:00:01Z');

  const status = await backend.status(KEY);
  assert.equal(status.terminated, false);
  assert.deepEqual(status.counter, { Task: 1, Artifact: 1 });
  assert.deepEqual(await backend.gates('agent', KEY), [{ participant: 'agent', key: KEY, enabled: true }]);
});

test('produce requires an enabled participant', async () => {
  const backend = new FakePactBackend({ enabled: false });
  await assert.rejects(
    backend.produce(KEY, 'agent', 'Artifact', [], 'a', 'tok-1', 't'),
    (error) => error instanceof PactBackendError && error.code === 'Disabled',
  );
});

test('consume removes a token and is idempotent per request id', async () => {
  const backend = new FakePactBackend();
  await backend.produce(KEY, 'agent', 'Artifact', [], 'a', 'tok-1', 't');

  const first = await backend.consume(KEY, 'agent', 'tok-1', 'req-1', 't1');
  const second = await backend.consume(KEY, 'agent', 'tok-1', 'req-1', 't2');
  assert.deepEqual(second, first);
  assert.deepEqual((await backend.status(KEY)).counter, {});
});

test('consume reports unknown tokens and idempotency conflicts', async () => {
  const backend = new FakePactBackend();
  await assert.rejects(
    backend.consume(KEY, 'agent', 'missing', null, 't'),
    (error) => error instanceof PactBackendError && error.code === 'UnknownToken',
  );

  await backend.produce(KEY, 'agent', 'Artifact', [], 'a', 'tok-1', 't');
  await backend.consume(KEY, 'agent', 'tok-1', 'req-1', 't');
  await backend.produce(KEY, 'agent', 'Artifact', [], 'a', 'tok-2', 't');
  await assert.rejects(
    backend.consume(KEY, 'agent', 'tok-2', 'req-1', 't'),
    (error) => error instanceof PactBackendError && error.code === 'IdempotencyConflict',
  );
});

test('duplicate token ids and closed backends are rejected', async () => {
  const backend = new FakePactBackend();
  await backend.seed(KEY, 'agent', 'Task', 't', 'tok-1', 't');
  await assert.rejects(
    backend.seed(KEY, 'agent', 'Task', 't', 'tok-1', 't'),
    (error) => error instanceof PactBackendError && error.code === 'DuplicateToken',
  );

  await backend.close();
  await assert.rejects(
    backend.enabled('agent', KEY),
    (error) => error instanceof PactBackendError && error.code === 'BackendUnavailable',
  );
});

test('invalid keys are rejected', async () => {
  const backend = new FakePactBackend();
  await assert.rejects(
    backend.enabled('agent', ''),
    (error) => error instanceof PactBackendError && error.code === 'InvalidKey',
  );
});
