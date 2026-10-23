import { readFileSync } from 'node:fs';
import { loadConfig } from './src/config.js';
import { FileSink, createObserver } from './src/observer.js';

/**
 * DSH host plugin: audit mirror for session events.
 *
 * This row never registers an AgentFactory; the factory is owned by the
 * `agent-loop` row overridden in cordis.patch.yml.
 */
export function apply(ctx) {
  const config = loadConfig(process.env, (path) => readFileSync(path, 'utf8'));
  const sink = config.mirrorDir === null ? null : new FileSink(config.mirrorDir);
  const observer = createObserver({ limit: config.mirrorLimit, sink });

  const onSessionEvent = (...args) => {
    const event = args.find((value) => value !== null
      && typeof value === 'object'
      && typeof value.type === 'string');
    if (event !== undefined) observer.record(event);
  };

  ctx.effect(() => {
    const off = ctx.on('session/event', onSessionEvent);
    return () => {
      if (typeof off === 'function') off();
      observer.close();
    };
  }, 'dsh-pact.observe()');
}
