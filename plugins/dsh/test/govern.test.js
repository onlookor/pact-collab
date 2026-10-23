import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeConfig } from '../src/config.js';
import { createGovernor } from '../src/govern.js';
import { FakePactBackend } from '../src/pact-backend.js';

const AGENT = {
  id: 'agent-1',
  session: { header: { id: 'sess/1', meta: {} } },
};

function governorFor({ mode = 'govern', preset = 'pact-flow', backend = new FakePactBackend(), extra = {} } = {}) {
  const config = normalizeConfig({ mode, ...extra });
  const audits = [];
  const errors = [];
  const governor = createGovernor({
    backend,
    config,
    presetFor: () => preset,
    audit: (entry) => audits.push(entry),
    now: () => '2026-10-22T00:00:00Z',
    onError: (error) => errors.push(error),
  });
  return { governor, audits, errors, config };
}

test('non-pact agents bypass every hook and call next exactly once', async () => {
  const { governor } = governorFor({ preset: 'default' });
  let calls = 0;
  const next = () => { calls += 1; return 'passed'; };

  assert.equal(await governor.onPreStep({ agent: AGENT }, next), 'passed');
  assert.equal(await governor.onPreTool({ agent: AGENT, name: 'shell' }, next), 'passed');
  assert.equal(await governor.onPostTool({ agent: AGENT, name: 'shell' }, {}, next), 'passed');
  await governor.onTurnStopping({ agent: AGENT });
  assert.equal(calls, 3);
});

test('pact agents are allowed when the counter is enabled', async () => {
  const { governor } = governorFor();
  let calls = 0;
  const next = () => { calls += 1; return 'passed'; };

  assert.equal(await governor.onPreStep({ agent: AGENT }, next), 'passed');
  assert.equal(await governor.onPreTool({ agent: AGENT, name: 'read_file' }, next), 'passed');
  assert.equal(calls, 2);
});

test('govern mode rejects a disabled participant before the step and tool', async () => {
  const backend = new FakePactBackend({ enabled: false });
  const { governor, audits } = governorFor({ backend });
  let calls = 0;
  const next = () => { calls += 1; return 'passed'; };

  assert.deepEqual(await governor.onPreStep({ agent: AGENT }, next), { kind: 'reject' });
  const toolDecision = await governor.onPreTool({ agent: AGENT, name: 'read_file' }, next);
  assert.equal(toolDecision.kind, 'deny');
  assert.match(toolDecision.reason, /not enabled/);
  assert.equal(calls, 0);
  assert.deepEqual(audits.map((entry) => entry.action), ['reject', 'deny']);
});

test('denyTools short-circuits without consulting the backend', async () => {
  const backend = new FakePactBackend();
  const { governor } = governorFor({ backend, extra: { denyTools: ['shell'] } });
  let calls = 0;
  const decision = await governor.onPreTool(
    { agent: AGENT, name: 'shell' },
    () => { calls += 1; },
  );
  assert.deepEqual(decision, { kind: 'deny', reason: 'pact: tool denied by policy' });
  assert.equal(calls, 0);
});

test('backend failures are fail-closed in govern and fail-open in observe', async () => {
  const closed = new FakePactBackend();
  await closed.close();

  const govern = governorFor({ backend: closed });
  let governCalls = 0;
  assert.deepEqual(
    await govern.governor.onPreStep({ agent: AGENT }, () => { governCalls += 1; }),
    { kind: 'reject' },
  );
  assert.equal(governCalls, 0);
  assert.equal(govern.errors.length, 1);

  const observe = governorFor({ mode: 'observe', backend: closed });
  let observeCalls = 0;
  assert.equal(
    await observe.governor.onPreStep({ agent: AGENT }, () => { observeCalls += 1; return 'passed'; }),
    'passed',
  );
  assert.equal(observeCalls, 1);
  assert.equal(observe.audits.at(-1).action, 'allow');
});

test('post-tool calls next once, preserves the acceptance, and records failure flags', async () => {
  const { governor, audits } = governorFor();
  let calls = 0;
  const acceptance = { kind: 'accept' };
  const next = async () => { calls += 1; return acceptance; };

  assert.equal(await governor.onPostTool({ agent: AGENT, name: 'read_file', callId: 'c1' }, {}, next), acceptance);
  await governor.onPostTool({ agent: AGENT, name: 'shell', callId: 'c2' }, { isError: true }, next);
  assert.equal(calls, 2);
  assert.deepEqual(audits.map((entry) => entry.ok), [true, false]);
  assert.deepEqual(audits.map((entry) => entry.callId), ['c1', 'c2']);
});

test('post-tool rethrows downstream errors after recording them', async () => {
  const { governor, audits } = governorFor();
  const failure = new Error('tool crashed');
  await assert.rejects(
    governor.onPostTool({ agent: AGENT, name: 'shell' }, {}, async () => { throw failure; }),
    failure,
  );
  assert.equal(audits.at(-1).ok, false);
});

test('turn-stopping steers only when termination is enabled', async () => {
  const steered = [];
  const agent = { ...AGENT, steer: async (message) => { steered.push(message); } };

  const on = governorFor({ backend: new FakePactBackend({ terminated: true }), extra: { stopOnTermination: true } });
  await on.governor.onTurnStopping({ agent });
  assert.deepEqual(steered, [{ content: '[pact] workflow terminated' }]);

  const off = governorFor({ backend: new FakePactBackend({ terminated: true }) });
  await off.governor.onTurnStopping({ agent });
  assert.equal(steered.length, 1);

  const alive = governorFor({ backend: new FakePactBackend({ terminated: false }), extra: { stopOnTermination: true } });
  await alive.governor.onTurnStopping({ agent });
  assert.equal(steered.length, 1);
});

test('turn-stopping swallows backend read failures', async () => {
  const closed = new FakePactBackend();
  await closed.close();
  const { governor, errors } = governorFor({ backend: closed, extra: { stopOnTermination: true } });
  const agent = { ...AGENT, steer: async () => { throw new Error('must not steer'); } };
  await governor.onTurnStopping({ agent });
  assert.equal(errors.length, 1);
});
