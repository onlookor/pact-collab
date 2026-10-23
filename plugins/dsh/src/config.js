import { isValidName } from './names.js';

export class PactConfigError extends Error {
  constructor(message) {
    super(message);
    this.name = 'PactConfigError';
  }
}

const DEFAULTS = Object.freeze({
  mode: 'observe',
  backend: 'fake',
  wasmModule: null,
  wasmOptions: Object.freeze({}),
  domain: 'dsh',
  defaultParticipant: 'agent',
  presetPrefix: 'pact-',
  participants: Object.freeze({}),
  mirrorDir: null,
  mirrorLimit: 256,
  denyTools: Object.freeze([]),
  stopOnTermination: false,
  failClosed: true,
  fakeEnabled: true,
});

const MODES = new Set(['observe', 'govern']);
const BACKENDS = new Set(['fake', 'wasm']);

function assertPlainObject(value, field) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new PactConfigError(`${field} must be an object`);
  }
}

/**
 * Validate one raw config object and fill defaults.
 * Unknown keys are ignored so a newer DSH row config does not break an older plugin.
 */
export function normalizeConfig(raw = {}) {
  if (raw === null || raw === undefined) raw = {};
  assertPlainObject(raw, 'config');

  const mode = raw.mode ?? DEFAULTS.mode;
  if (!MODES.has(mode)) {
    throw new PactConfigError(`mode must be one of ${[...MODES].join(', ')}`);
  }

  const backend = raw.backend ?? DEFAULTS.backend;
  if (!BACKENDS.has(backend)) {
    throw new PactConfigError(`backend must be one of ${[...BACKENDS].join(', ')}`);
  }

  const domain = raw.domain ?? DEFAULTS.domain;
  if (!isValidName(domain)) {
    throw new PactConfigError('domain must match [A-Za-z0-9_-]{1,128}');
  }

  const defaultParticipant = raw.defaultParticipant ?? DEFAULTS.defaultParticipant;
  if (!isValidName(defaultParticipant)) {
    throw new PactConfigError('defaultParticipant must match [A-Za-z0-9_-]{1,128}');
  }

  const presetPrefix = raw.presetPrefix ?? DEFAULTS.presetPrefix;
  if (typeof presetPrefix !== 'string' || presetPrefix.length === 0) {
    throw new PactConfigError('presetPrefix must be a non-empty string');
  }

  const participants = raw.participants ?? DEFAULTS.participants;
  assertPlainObject(participants, 'participants');
  // Null prototype: a JSON key named "__proto__" must stay a data key.
  const normalizedParticipants = Object.create(null);
  for (const [preset, participant] of Object.entries(participants)) {
    if (typeof preset !== 'string' || preset.length === 0) {
      throw new PactConfigError('participants keys must be non-empty strings');
    }
    if (!isValidName(participant)) {
      throw new PactConfigError(`participant for preset "${preset}" must match [A-Za-z0-9_-]{1,128}`);
    }
    normalizedParticipants[preset] = participant;
  }

  const mirrorDir = raw.mirrorDir ?? DEFAULTS.mirrorDir;
  if (mirrorDir !== null && (typeof mirrorDir !== 'string' || mirrorDir.length === 0)) {
    throw new PactConfigError('mirrorDir must be null or a non-empty string');
  }

  const mirrorLimit = raw.mirrorLimit ?? DEFAULTS.mirrorLimit;
  if (!Number.isInteger(mirrorLimit) || mirrorLimit < 1 || mirrorLimit > 4096) {
    throw new PactConfigError('mirrorLimit must be an integer in 1..4096');
  }

  const denyTools = raw.denyTools ?? DEFAULTS.denyTools;
  if (!Array.isArray(denyTools) || denyTools.some((tool) => typeof tool !== 'string' || tool.length === 0)) {
    throw new PactConfigError('denyTools must be an array of non-empty strings');
  }

  const stopOnTermination = raw.stopOnTermination ?? DEFAULTS.stopOnTermination;
  if (typeof stopOnTermination !== 'boolean') {
    throw new PactConfigError('stopOnTermination must be a boolean');
  }

  const failClosed = raw.failClosed ?? DEFAULTS.failClosed;
  if (typeof failClosed !== 'boolean') {
    throw new PactConfigError('failClosed must be a boolean');
  }

  const fakeEnabled = raw.fakeEnabled ?? DEFAULTS.fakeEnabled;
  if (typeof fakeEnabled !== 'boolean') {
    throw new PactConfigError('fakeEnabled must be a boolean');
  }

  const wasmModule = raw.wasmModule ?? DEFAULTS.wasmModule;
  if (wasmModule !== null && (typeof wasmModule !== 'string' || wasmModule.length === 0)) {
    throw new PactConfigError('wasmModule must be null or a non-empty string');
  }
  if (backend === 'wasm' && wasmModule === null) {
    throw new PactConfigError('wasmModule is required when backend is "wasm"');
  }

  // Backend-specific fatal options (store root, wasm out-dir, declaration...).
  // The plugin only requires a plain object; the module validates its own keys.
  const wasmOptions = raw.wasmOptions === undefined ? DEFAULTS.wasmOptions : raw.wasmOptions;
  assertPlainObject(wasmOptions, 'wasmOptions');

  return {
    mode,
    backend,
    wasmModule,
    wasmOptions: Object.freeze({ ...wasmOptions }),
    domain,
    defaultParticipant,
    presetPrefix,
    participants: Object.freeze(normalizedParticipants),
    mirrorDir,
    mirrorLimit,
    denyTools: Object.freeze([...denyTools]),
    stopOnTermination,
    // A governed loop must stay conservative: fail-closed is not optional.
    failClosed: mode === 'govern' ? true : failClosed,
    // Deterministic backend double: `false` makes every participant disabled,
    // which lets an operator rehearse reject paths without a real PACT engine.
    fakeEnabled,
  };
}

/**
 * Load config from PACT_DSH_CONFIG (a file path or inline JSON) or return defaults.
 * A missing/empty value is not an error; malformed JSON is.
 */
export function loadConfig(env = process.env, readFile = null) {
  const value = env?.PACT_DSH_CONFIG;
  if (value === undefined || value === null || value === '') return normalizeConfig({});

  let text = value;
  if (!value.trimStart().startsWith('{')) {
    if (typeof readFile !== 'function') {
      throw new PactConfigError('PACT_DSH_CONFIG points to a file but no reader is available');
    }
    text = readFile(value);
  }

  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new PactConfigError(`PACT_DSH_CONFIG is not valid JSON: ${error.message}`);
  }
  return normalizeConfig(parsed);
}
