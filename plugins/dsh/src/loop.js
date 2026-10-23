import { createGovernor } from './govern.js';
import { instanceFromSession, isPactPreset } from './names.js';

/**
 * Build a DSH AgentLoop subclass that owns the AgentFactory slot but delegates
 * mechanical turn execution to the official loop.
 *
 * Only agents created with a `pact-*` preset (and only in govern mode) get
 * PACT hooks. Every other agent is handed to `super` untouched.
 *
 * @param {Function} Base - the official AgentLoop class.
 * @param {object} deps
 * @param {object} deps.config - normalized PACT config.
 * @param {(key: string) => object} deps.backendFor - backend factory per instance key.
 * @param {Function} [deps.audit] - optional audit sink for governance decisions.
 */
export function createPactLoop(Base, { config, backendFor, audit = () => {} } = {}) {
  if (typeof Base !== 'function') throw new TypeError('Base must be the DSH AgentLoop class');
  if (typeof backendFor !== 'function') throw new TypeError('backendFor must be a function');

  return class PactAgentLoop extends Base {
    static Config = Base.Config;

    /** Registration map, created lazily in case the base loop creates an agent during construction. */
    pactRegistry() {
      this.pactPresets ??= new WeakMap();
      return this.pactPresets;
    }

    /** Instance map keyed by agent, so every hook agrees on one PACT instance. */
    pactInstanceRegistry() {
      this.pactInstances ??= new WeakMap();
      return this.pactInstances;
    }

    /** Preset lookup: explicit registry first, then DSH session metadata. */
    pactPresetFor(agent) {
      const registered = agent && this.pactRegistry().get(agent);
      if (typeof registered === 'string') return registered;
      const fromOptions = agent?.options?.agentPreset;
      if (typeof fromOptions === 'string') return fromOptions;
      const fromSession = agent?.session?.header?.meta?.agentPreset;
      return typeof fromSession === 'string' ? fromSession : '';
    }

    /** Instance lookup: explicit registry first, then session/agent identities. */
    pactInstanceFor(agent) {
      const registered = agent && this.pactInstanceRegistry().get(agent);
      if (typeof registered === 'string') return registered;
      const sessionId = agent?.session?.header?.id ?? agent?.session?.id ?? agent?.id;
      return instanceFromSession(sessionId);
    }

    pactInstall(agentCtx, agent, preset, sessionId) {
      this.pactRegistry().set(agent, preset);
      this.pactInstanceRegistry().set(agent, instanceFromSession(sessionId ?? this.pactInstanceFor(agent)));
      const governor = createGovernor({
        backend: backendFor(`${config.domain}/${this.pactInstanceFor(agent)}`),
        config,
        presetFor: (target) => this.pactPresetFor(target),
        keyFor: (target) => `${config.domain}/${this.pactInstanceFor(target)}`,
        audit,
      });
      agentCtx.effect(() => {
        const offs = [
          agentCtx.on('agent/pre-step', (payload, next) => governor.onPreStep(payload ?? { agent }, next)),
          agentCtx.on('tools/pre-execute', (exec, next) => governor.onPreTool(exec, next)),
          agentCtx.on('tools/post-execute', (exec, result, next) => governor.onPostTool(exec, result, next)),
          agentCtx.on('agent/turn-stopping', (payload) => governor.onTurnStopping(payload ?? { agent })),
        ];
        return () => {
          for (const off of offs.reverse()) {
            if (typeof off === 'function') off();
          }
        };
      }, 'dsh-pact.governance()');
    }

    async createAgent(ownerCtx, options) {
      const opts = options ?? {};
      const preset = opts.meta?.agentPreset ?? '';
      if (config.mode !== 'govern' || !isPactPreset(preset, config.presetPrefix)) {
        return super.createAgent(ownerCtx, options);
      }

      const originalSetup = opts.setup;
      const setup = async (agentCtx, agent) => {
        const commit = typeof originalSetup === 'function'
          ? await originalSetup(agentCtx, agent)
          : undefined;
        this.pactInstall(agentCtx, agent, preset, opts.sessionId);
        return commit;
      };
      return super.createAgent(ownerCtx, { ...opts, setup });
    }

    async resume(ownerCtx, options) {
      const handle = await super.resume(ownerCtx, options);
      if (config.mode !== 'govern') return handle;

      const agent = handle?.agent;
      const preset = this.pactPresetFor(agent) || options?.agentPreset || '';
      if (isPactPreset(preset, config.presetPrefix) && agent?.ctx) {
        // Best effort: hooks are installed after publication, so the resumed
        // agent's first already-in-flight step is not gated.
        this.pactInstall(agent.ctx, agent, preset, options?.sessionId);
      }
      return handle;
    }
  };
}
