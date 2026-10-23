import test from 'node:test';
import assert from 'node:assert/strict';
import { PactConfigError, loadConfig, normalizeConfig } from '../src/config.js';

test('defaults are observe/fake with fail-closed', () => {
  const config = normalizeConfig();
  assert.equal(config.mode, 'observe');
  assert.equal(config.backend, 'fake');
  assert.equal(config.domain, 'dsh');
  assert.equal(config.defaultParticipant, 'agent');
  assert.equal(config.presetPrefix, 'pact-');
  assert.equal(config.mirrorDir, null);
  assert.equal(config.mirrorLimit, 256);
  assert.deepEqual(config.denyTools, []);
  assert.equal(config.failClosed, true);
  assert.equal(config.fakeEnabled, true);
});

test('govern mode forces fail-closed even when explicitly disabled', () => {
  const config = normalizeConfig({ mode: 'govern', failClosed: false });
  assert.equal(config.failClosed, true);
});

test('rejects unknown mode, backend, and malformed names', () => {
  assert.throws(() => normalizeConfig({ mode: 'replace' }), PactConfigError);
  assert.throws(() => normalizeConfig({ backend: 'daemon' }), PactConfigError);
  assert.throws(() => normalizeConfig({ domain: '../etc' }), PactConfigError);
  assert.throws(() => normalizeConfig({ defaultParticipant: 'a b' }), PactConfigError);
  assert.throws(() => normalizeConfig({ participants: { 'pact-x': 'bad name' } }), PactConfigError);
});

test('validates mirror and tool settings', () => {
  assert.throws(() => normalizeConfig({ mirrorLimit: 0 }), PactConfigError);
  assert.throws(() => normalizeConfig({ mirrorLimit: 4097 }), PactConfigError);
  assert.throws(() => normalizeConfig({ mirrorLimit: 1.5 }), PactConfigError);
  assert.throws(() => normalizeConfig({ denyTools: ['ok', ''] }), PactConfigError);
  assert.throws(() => normalizeConfig({ stopOnTermination: 'yes' }), PactConfigError);
  assert.throws(() => normalizeConfig({ fakeEnabled: 'no' }), PactConfigError);
  const config = normalizeConfig({ mirrorDir: '/tmp/pact', denyTools: ['shell'], fakeEnabled: false });
  assert.equal(config.mirrorDir, '/tmp/pact');
  assert.deepEqual(config.denyTools, ['shell']);
  assert.equal(config.fakeEnabled, false);
});

test('participants are stored without a prototype', () => {
  const config = normalizeConfig({ participants: JSON.parse('{"__proto__":"agent"}') });
  assert.equal(Object.getPrototypeOf(config.participants), null);
  assert.equal(config.participants.__proto__, 'agent');
  assert.equal(Object.getPrototypeOf({}), Object.prototype, 'the global prototype is untouched');
});

test('wasm backend requires a module', () => {
  assert.throws(() => normalizeConfig({ backend: 'wasm' }), PactConfigError);
  assert.equal(normalizeConfig({ backend: 'wasm', wasmModule: 'pact-wasm' }).wasmModule, 'pact-wasm');
});

test('wasmOptions is an optional plain object passed to the host', () => {
  assert.deepEqual(normalizeConfig({ backend: 'wasm', wasmModule: 'm' }).wasmOptions, {});
  const config = normalizeConfig({
    backend: 'wasm',
    wasmModule: 'm',
    wasmOptions: { storeRoot: '/tmp/store', declaration: 'types { a }' },
  });
  assert.equal(config.wasmOptions.storeRoot, '/tmp/store');
  assert.throws(() => normalizeConfig({ wasmOptions: [] }), PactConfigError);
  assert.throws(() => normalizeConfig({ wasmOptions: null }), PactConfigError);
  assert.throws(() => normalizeConfig({ wasmOptions: 'x' }), PactConfigError);
});

test('loadConfig returns defaults without the env var', () => {
  assert.equal(loadConfig({}).mode, 'observe');
  assert.equal(loadConfig({ PACT_DSH_CONFIG: '' }).mode, 'observe');
});

test('loadConfig reads inline JSON and file paths', () => {
  const inline = loadConfig({ PACT_DSH_CONFIG: '{"mode":"govern"}' });
  assert.equal(inline.mode, 'govern');

  const fromFile = loadConfig(
    { PACT_DSH_CONFIG: '/etc/pact/dsh.json' },
    (path) => {
      assert.equal(path, '/etc/pact/dsh.json');
      return '{"mode":"govern","domain":"work"}';
    },
  );
  assert.equal(fromFile.domain, 'work');
});

test('loadConfig rejects malformed JSON', () => {
  assert.throws(() => loadConfig({ PACT_DSH_CONFIG: '{nope' }), PactConfigError);
  assert.throws(
    () => loadConfig({ PACT_DSH_CONFIG: '/tmp/pact.json' }, () => 'not json'),
    PactConfigError,
  );
});
