import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeConfig } from '../src/config.js';
import { createPactLoop } from '../src/loop.js';
import { FakePactBackend } from '../src/pact-backend.js';

function makeAgentCtx() {
  const hooks = new Map();
  const effects = [];
  return {
    hooks,
    effects,
    on(name, handler) {
      const handlers = hooks.get(name) ?? [];
      handlers.push(handler);
      hooks.set(name, handlers);
      return () => handlers.splice(handlers.indexOf(handler), 1);
    },
    effect(fn) {
      const disposer = fn();
      effects.push(disposer);
      return disposer;
    },
  };
}

class FakeBase {
  static Config = { marker: 'base' };

  constructor(ctx, config) {
    this.ctx = ctx;
    this.config = config;
    this.calls = [];
  }

  async createAgent(ownerCtx, options = {}) {
    this.calls.push('createAgent');
    this.lastOptions = options;
    this.agentCtx = makeAgentCtx();
    this.agent = {
      id: 'agent-1',
      session: { header: { id: 'sess-1', meta: { agentPreset: options.meta?.agentPreset } } },
    };
    this.setupResult = typeof options.setup === 'function'
      ? await options.setup(this.agentCtx, this.agent)
      : undefined;
    return { agent: this.agent };
  }

  async resume(ownerCtx, options = {}) {
    this.calls.push('resume');
    const agent = {
      id: 'agent-1',
      session: { header: { id: 'sess-1', meta: { agentPreset: options.agentPreset } } },
      ctx: makeAgentCtx(),
    };
    return { agent };
  }
}

function build({ mode = 'govern', backend = new FakePactBackend(), audit = () => {} } = {}) {
  const config = normalizeConfig({ mode });
  const Loop = createPactLoop(FakeBase, { config, backendFor: () => backend, audit });
  return { Loop, loop: new Loop({}, {}), config };
}

test('non-pact presets delegate untouched', async () => {
  const { loop } = build();
  const setup = async () => {};
  const options = { meta: { agentPreset: 'default' }, setup };

  await loop.createAgent({}, options);
  assert.equal(loop.lastOptions, options);
  assert.equal(loop.agentCtx.hooks.size, 0);
  assert.equal(loop.agentCtx.effects.length, 0);
});

test('observe mode never installs governance, even for pact presets', async () => {
  const { loop } = build({ mode: 'observe' });
  const options = { meta: { agentPreset: 'pact-flow' } };
  await loop.createAgent({}, options);
  assert.equal(loop.lastOptions, options);
  assert.equal(loop.agentCtx.hooks.size, 0);
});

test('govern mode composes setup and installs four agent-scoped hooks', async () => {
  const { loop } = build();
  const commit = { commit: () => {} };
  const options = {
    meta: { agentPreset: 'pact-flow' },
    setup: async (agentCtx) => {
      assert.equal(agentCtx.hooks.size, 0, 'original setup runs before PACT hooks');
      return commit;
    },
  };

  await loop.createAgent({}, options);
  assert.notEqual(loop.lastOptions, options);
  assert.equal(loop.setupResult, commit, 'the caller commit is preserved');
  assert.deepEqual([...loop.agentCtx.hooks.keys()].sort(), [
    'agent/pre-step',
    'agent/turn-stopping',
    'tools/post-execute',
    'tools/pre-execute',
  ]);
  assert.equal(loop.agentCtx.effects.length, 1);
  assert.equal(loop.pactPresetFor(loop.agent), 'pact-flow');
});

test('governed hooks enforce the gate for the created agent', async () => {
  const backend = new FakePactBackend({ enabled: false });
  const { loop } = build({ backend });
  await loop.createAgent({}, { meta: { agentPreset: 'pact-flow' } });

  const preStep = loop.agentCtx.hooks.get('agent/pre-step')[0];
  let nextCalls = 0;
  const decision = await preStep({ agent: loop.agent }, () => { nextCalls += 1; });
  assert.deepEqual(decision, { kind: 'reject' });
  assert.equal(nextCalls, 0);

  const preTool = loop.agentCtx.hooks.get('tools/pre-execute')[0];
  const denial = await preTool({ agent: loop.agent, name: 'shell' }, () => { nextCalls += 1; });
  assert.equal(denial.kind, 'deny');
  assert.equal(nextCalls, 0);
});

test('the factory sessionId becomes one sanitized instance for every hook', async () => {
  const keys = [];
  const backend = new FakePactBackend({ enabled: false });
  const config = normalizeConfig({ mode: 'govern' });
  const Loop = createPactLoop(FakeBase, {
    config,
    backendFor: (key) => { keys.push(key); return backend; },
    audit: (entry) => keys.push(entry.key),
  });
  const loop = new Loop({}, {});
  await loop.createAgent({}, { sessionId: 'sess/42', meta: { agentPreset: 'pact-flow' } });

  assert.equal(loop.pactInstanceFor(loop.agent), 'sess_42');
  assert.deepEqual(keys, ['dsh/sess_42']);

  // A hook for the same agent must resolve to the same instance key.
  const preStep = loop.agentCtx.hooks.get('agent/pre-step')[0];
  await preStep({ agent: loop.agent }, () => 'passed');
  assert.equal(keys.at(-1), 'dsh/sess_42');
});

test('resume installs hooks for a pact session and skips others', async () => {
  const { loop } = build();
  const resumed = await loop.resume({}, { agentPreset: 'pact-resume' });
  assert.equal(resumed.agent.ctx.hooks.size, 4);

  const plain = await loop.resume({}, { agentPreset: 'default' });
  assert.equal(plain.agent.ctx.hooks.size, 0);
});

test('the subclass reuses the base Config schema', () => {
  const { Loop } = build();
  assert.equal(Loop.Config, FakeBase.Config);
});
