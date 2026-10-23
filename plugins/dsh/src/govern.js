import { instanceFromSession, isPactPreset, participantFor } from './names.js';

export const STOP_MESSAGE = '[pact] workflow terminated';

function safePreset(presetFor, agent) {
  try {
    const value = presetFor?.(agent);
    return typeof value === 'string' ? value : '';
  } catch {
    return '';
  }
}

function failed(result) {
  return result?.isError === true || (result?.error !== undefined && result?.error !== null);
}

/**
 * Build the four DSH hook handlers used by a governed agent.
 *
 * The handlers never throw: backend errors are reported through `onError` and
 * resolved by policy (observe passes, govern is fail-closed).
 */
export function createGovernor({
  backend,
  config,
  presetFor,
  keyFor = null,
  audit = () => {},
  now = () => new Date().toISOString(),
  onError = () => {},
  stopMessage = STOP_MESSAGE,
}) {
  const resolve = (agent) => {
    const preset = safePreset(presetFor, agent);
    if (typeof keyFor === 'function') {
      return {
        preset,
        governed: isPactPreset(preset, config.presetPrefix),
        participant: participantFor(preset, config),
        key: keyFor(agent),
      };
    }
    const sessionId = agent?.session?.header?.id
      ?? agent?.session?.id
      ?? agent?.id;
    return {
      preset,
      governed: isPactPreset(preset, config.presetPrefix),
      participant: participantFor(preset, config),
      key: `${config.domain}/${instanceFromSession(sessionId)}`,
    };
  };

  const enabledOrNull = async (target) => {
    try {
      return await backend.enabled(target.participant, target.key);
    } catch (error) {
      onError(error);
      return null;
    }
  };

  const allow = () => ({ ok: true });
  const deny = (reason) => ({ ok: false, reason });

  const decide = async (target, action) => {
    const enabled = await enabledOrNull(target);
    if (enabled === true) return allow();
    if (enabled === false) {
      return deny(`pact: participant "${target.participant}" is not enabled`);
    }
    // Backend failure: observe mode keeps the harness running, govern is conservative.
    const passThrough = config.mode === 'observe' || config.failClosed !== true;
    audit({ at: now(), event: `${action}-backend-error`, key: target.key, participant: target.participant, action: passThrough ? 'allow' : 'deny' });
    return passThrough ? allow() : deny('pact: gate unavailable');
  };

  return {
    async onPreStep(payload, next) {
      const agent = payload?.agent;
      const target = resolve(agent);
      if (!target.governed) return next();

      const decision = await decide(target, 'pre-step');
      audit({
        at: now(),
        event: 'pre-step',
        key: target.key,
        participant: target.participant,
        action: decision.ok ? 'allow' : 'reject',
      });
      if (!decision.ok) return { kind: 'reject' };
      return next();
    },

    async onPreTool(exec, next) {
      const agent = exec?.agent;
      const target = resolve(agent);
      if (!target.governed) return next();

      const tool = typeof exec?.name === 'string' ? exec.name : '';
      if (tool !== '' && config.denyTools.includes(tool)) {
        audit({ at: now(), event: 'tool-denied', key: target.key, participant: target.participant, tool, action: 'deny' });
        return { kind: 'deny', reason: 'pact: tool denied by policy' };
      }

      const decision = await decide(target, 'pre-tool');
      audit({
        at: now(),
        event: 'pre-tool',
        key: target.key,
        participant: target.participant,
        tool,
        action: decision.ok ? 'allow' : 'deny',
      });
      if (!decision.ok) return { kind: 'deny', reason: decision.reason };
      return next();
    },

    async onPostTool(exec, result, next) {
      const target = resolve(exec?.agent);
      if (!target.governed) return next();

      let acceptance;
      try {
        acceptance = await next();
      } catch (error) {
        audit({
          at: now(),
          event: 'post-tool',
          key: target.key,
          participant: target.participant,
          tool: typeof exec?.name === 'string' ? exec.name : '',
          callId: typeof exec?.callId === 'string' ? exec.callId : undefined,
          ok: false,
        });
        throw error;
      }
      audit({
        at: now(),
        event: 'post-tool',
        key: target.key,
        participant: target.participant,
        tool: typeof exec?.name === 'string' ? exec.name : '',
        callId: typeof exec?.callId === 'string' ? exec.callId : undefined,
        ok: !failed(result),
      });
      return acceptance;
    },

    async onTurnStopping(payload) {
      const agent = payload?.agent;
      const target = resolve(agent);
      if (!target.governed || !config.stopOnTermination) return;

      try {
        const status = await backend.status(target.key);
        if (status?.terminated === true) {
          await agent?.steer?.({ content: stopMessage });
          audit({ at: now(), event: 'turn-stopped', key: target.key, participant: target.participant, action: 'steer' });
        }
      } catch (error) {
        // Turn close must never be blocked by a PACT read failure.
        onError(error);
      }
    },
  };
}
