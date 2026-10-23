import test from 'node:test';
import assert from 'node:assert/strict';
import { instanceFromSession, isPactPreset, isValidName, participantFor } from '../src/names.js';

test('isPactPreset only matches explicit string prefixes', () => {
  assert.equal(isPactPreset('pact-flow'), true);
  assert.equal(isPactPreset('pact-flow', 'flow-'), false);
  assert.equal(isPactPreset('PACT-flow'), false);
  assert.equal(isPactPreset('default'), false);
  assert.equal(isPactPreset(undefined), false);
  assert.equal(isPactPreset(null), false);
  assert.equal(isPactPreset(42), false);
});

test('instanceFromSession sanitizes path-unsafe characters', () => {
  assert.equal(instanceFromSession('a/b'), 'a_b');
  assert.equal(instanceFromSession('../../etc/passwd'), '______etc_passwd');
  assert.equal(instanceFromSession('sess-01_AB'), 'sess-01_AB');
  assert.equal(instanceFromSession(''), 'session');
  assert.equal(instanceFromSession(undefined), 'session');
  assert.equal(instanceFromSession('x'.repeat(300)).length, 128);
});

test('participantFor prefers the preset mapping', () => {
  const config = { defaultParticipant: 'worker', participants: { 'pact-review': 'reviewer' } };
  assert.equal(participantFor('pact-review', config), 'reviewer');
  assert.equal(participantFor('pact-build', config), 'worker');
  assert.equal(participantFor(undefined, config), 'worker');
  assert.equal(participantFor('pact-x', {}), 'agent');
});

test('isValidName enforces the 1..128 safe charset', () => {
  assert.equal(isValidName('ok-name_1'), true);
  assert.equal(isValidName(''), false);
  assert.equal(isValidName('a.b'), false);
  assert.equal(isValidName('a'.repeat(129)), false);
  assert.equal(isValidName(1), false);
});
