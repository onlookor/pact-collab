import { PactBackendError } from './pact-backend.js';

const UNAVAILABLE_HINT = 'PACT WASM backend is not available: build the pact-wasm package and expose openPactHost(options)';

/**
 * Adapter for a `pact-wasm` host (see `crates/pact-wasm/js/pact-host.js`).
 *
 * The WASM host is bound to one `(domain, instance)` pair, so one adapter
 * instance serves exactly one PACT key. `module` must expose
 * `openPactHost(options)` (or a default export with that function) returning a
 * PactHost-like object: enabled(participant), gates(participant),
 * status(), seed(...), produce(...), consume(...), close().
 *
 * This layer is intentionally lazy: importing it must not require the WASM
 * artifact to exist, and a missing module surfaces as BackendUnavailable only
 * when a governed call actually needs it.
 */
export function createWasmBackend({ module = null, options = {}, loadHost = null } = {}) {
  if (module === null && typeof loadHost !== 'function') {
    throw new PactBackendError('BackendUnavailable', UNAVAILABLE_HINT);
  }

  let hostPromise = null;
  let closed = false;
  const host = () => {
    hostPromise ??= (async () => {
      if (typeof loadHost === 'function') return await loadHost();
      const loaded = await import(module);
      const open = loaded.openPactHost
        ?? (loaded.default && typeof loaded.default.openPactHost === 'function' ? loaded.default.openPactHost : null);
      if (typeof open !== 'function') {
        throw new PactBackendError('BackendUnavailable', UNAVAILABLE_HINT);
      }
      return await open(options);
    })();
    return hostPromise;
  };

  const call = async (method, ...args) => {
    if (closed) {
      throw new PactBackendError('BackendUnavailable', 'PACT WASM backend is closed');
    }
    try {
      const target = await host();
      if (typeof target?.[method] !== 'function') {
        throw new PactBackendError('BackendUnavailable', `PACT host does not implement ${method}()`);
      }
      return await target[method](...args);
    } catch (error) {
      if (error instanceof PactBackendError) throw error;
      throw new PactBackendError('BackendUnavailable', error?.message ?? String(error));
    }
  };

  // The host is already scoped to one key; `key` is accepted for contract parity.
  return {
    // `PactHost.enabled` returns the Rule-W production options; the plugin
    // contract is a boolean ("this participant has something to do now").
    enabled: async (participant) => {
      const options = await call('enabled', participant);
      return Array.isArray(options) ? options.length > 0 : Boolean(options);
    },
    gates: (participant) => call('gates', participant),
    status: () => call('status'),
    seed: (_key, participant, type, thingRef, tokenId, ts) => call('seed', participant, type, thingRef, tokenId, ts),
    produce: (_key, participant, type, predecessors, thingRef, tokenId, ts) => call('produce', participant, type, predecessors, thingRef, tokenId, ts),
    consume: (_key, participant, tokenId, requestId, ts) => call('consume', participant, tokenId, requestId, ts),
    close: async () => {
      if (closed) return;
      closed = true;
      if (hostPromise === null) return;
      const target = await hostPromise.catch(() => null);
      if (target && typeof target.close === 'function') await target.close();
    },
  };
}
