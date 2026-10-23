export const NAME_RE = /^[A-Za-z0-9_-]{1,128}$/;

/** True only for an explicit string with the configured preset prefix. */
export function isPactPreset(preset, prefix = 'pact-') {
  return typeof preset === 'string' && preset.startsWith(prefix);
}

/**
 * Derive a PACT instance key from a DSH session id.
 * Session ids may contain characters that are unsafe as path segments
 * (e.g. "/" or ".") and are therefore replaced before the bounded truncation.
 */
export function instanceFromSession(sessionId) {
  const raw = typeof sessionId === 'string' ? sessionId : '';
  const cleaned = raw.replace(/[^A-Za-z0-9_-]/g, '_').slice(0, 128);
  return cleaned || 'session';
}

/** Resolve the PACT participant for a preset, falling back to the configured default. */
export function participantFor(preset, config) {
  if (typeof preset === 'string'
    && config?.participants
    && Object.prototype.hasOwnProperty.call(config.participants, preset)) {
    return config.participants[preset];
  }
  return config?.defaultParticipant ?? 'agent';
}

export function isValidName(value) {
  return typeof value === 'string' && NAME_RE.test(value);
}
