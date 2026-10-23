/**
 * PACT backend contract for the DSH plugin.
 *
 * A backend is keyed by `domain/instance`; the plugin never assumes a single
 * instance. Implementations are responsible for PACT semantics (gates, tokens,
 * idempotency). This module ships a deterministic in-memory double used by the
 * unit tests; the WASM adapter lives in `wasm-backend.js`.
 */

export class PactBackendError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'PactBackendError';
    this.code = code;
  }
}

function requireKey(key) {
  if (typeof key !== 'string' || key.length === 0) {
    throw new PactBackendError('InvalidKey', 'key must be a non-empty "domain/instance" string');
  }
}

function requireName(value, field) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new PactBackendError('InvalidRequest', `${field} must be a non-empty string`);
  }
}

/**
 * Deterministic backend double.
 *
 * It intentionally models only the surface the plugin consumes:
 * participant enablement, token presence, and request-id idempotency.
 * It is not a PACT engine and makes no gate-expression judgement.
 */
export class FakePactBackend {
  constructor({ participants = ['agent'], enabled = true, terminated = false, gate = null } = {}) {
    this.participants = new Set(participants);
    this._enabled = enabled;
    this._terminated = terminated;
    this._gate = gate;
    this._tokens = new Map(); // key -> Map(tokenId, token)
    this._requests = new Map(); // `${key}\u0000${requestId}` -> { tokenId, result }
    this._closed = false;
  }

  setEnabled(value) {
    this._enabled = Boolean(value);
  }

  async enabled(participant, key) {
    this._assertOpen();
    requireKey(key);
    requireName(participant, 'participant');
    if (typeof this._gate === 'function') {
      const allowed = await this._gate({ participant, key });
      if (!allowed) return false;
    }
    return this._enabled && this.participants.has(participant);
  }

  async gates(participant, key) {
    this._assertOpen();
    requireKey(key);
    requireName(participant, 'participant');
    return [{ participant, key, enabled: await this.enabled(participant, key) }];
  }

  async status(key) {
    this._assertOpen();
    requireKey(key);
    const tokens = this._tokens.get(key);
    const counter = {};
    if (tokens) {
      for (const token of tokens.values()) {
        counter[token.type] = (counter[token.type] ?? 0) + 1;
      }
    }
    return { terminated: this._terminated, counter };
  }

  async seed(key, participant, type, thingRef, tokenId, ts) {
    return this._add(key, participant, type, thingRef, tokenId, ts, []);
  }

  async produce(key, participant, type, predecessors, thingRef, tokenId, ts) {
    return this._add(key, participant, type, thingRef, tokenId, ts, predecessors ?? []);
  }

  async _add(key, participant, type, thingRef, tokenId, ts, predecessors) {
    if (!(await this.enabled(participant, key))) {
      throw new PactBackendError('Disabled', `participant "${participant}" is not enabled`);
    }
    return this._insert(key, participant, type, thingRef, tokenId, ts, predecessors);
  }

  async consume(key, participant, tokenId, requestId, ts) {
    this._assertOpen();
    requireKey(key);
    requireName(participant, 'participant');
    requireName(tokenId, 'tokenId');

    if (requestId !== null && requestId !== undefined) {
      const requestKey = `${key}\u0000${requestId}`;
      const previous = this._requests.get(requestKey);
      if (previous !== undefined) {
        if (previous.tokenId !== tokenId) {
          throw new PactBackendError(
            'IdempotencyConflict',
            `request_id "${requestId}" was already used for another token`,
          );
        }
        return previous.result;
      }
    }

    if (!(await this.enabled(participant, key))) {
      throw new PactBackendError('Disabled', `participant "${participant}" is not enabled`);
    }

    const tokens = this._tokens.get(key);
    const token = tokens?.get(tokenId);
    if (token === undefined) {
      throw new PactBackendError('UnknownToken', `token "${tokenId}" does not exist`);
    }
    tokens.delete(tokenId);
    const result = { token_id: tokenId, type: token.type, participant, thing_ref: token.thingRef, consumed_at: ts };
    if (requestId !== null && requestId !== undefined) {
      this._requests.set(`${key}\u0000${requestId}`, { tokenId, result });
    }
    return result;
  }

  async close() {
    this._closed = true;
  }

  _insert(key, participant, type, thingRef, tokenId, ts, predecessors) {
    this._assertOpen();
    requireKey(key);
    requireName(participant, 'participant');
    requireName(type, 'type');
    requireName(tokenId, 'tokenId');

    const tokens = this._tokens.get(key) ?? new Map();
    if (tokens.has(tokenId)) {
      throw new PactBackendError('DuplicateToken', `token "${tokenId}" already exists`);
    }
    const token = { tokenId, type, participant, thingRef, predecessors, ts };
    tokens.set(tokenId, token);
    this._tokens.set(key, tokens);
    return { token_id: tokenId, type, participant, thing_ref: thingRef, predecessors, ts };
  }

  _assertOpen() {
    if (this._closed) {
      throw new PactBackendError('BackendUnavailable', 'backend is closed');
    }
  }
}
