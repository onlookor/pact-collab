import { readFileSync } from 'node:fs';
import { AgentLoop } from '@deepseek-ai/dsh-agent-loop';
import { loadConfig } from './src/config.js';
import { createPactLoop } from './src/loop.js';
import { FakePactBackend } from './src/pact-backend.js';
import { createWasmBackend } from './src/wasm-backend.js';

const config = loadConfig(process.env, (path) => readFileSync(path, 'utf8'));

const participants = new Set([config.defaultParticipant, ...Object.values(config.participants)]);
let fakeBackend = null;
const wasmBackends = new Map();

function backendFor(key) {
  if (config.backend === 'wasm') {
    let backend = wasmBackends.get(key);
    if (backend === undefined) {
      const separator = key.indexOf('/');
      const domain = separator === -1 ? key : key.slice(0, separator);
      const instance = separator === -1 ? 'default' : key.slice(separator + 1);
      backend = createWasmBackend({
        module: config.wasmModule,
        options: { ...config.wasmOptions, domain, instance },
      });
      wasmBackends.set(key, backend);
    }
    return backend;
  }
  if (fakeBackend === null) {
    fakeBackend = new FakePactBackend({
      participants: [...participants],
      enabled: config.fakeEnabled,
    });
  }
  return fakeBackend;
}

export default createPactLoop(AgentLoop, { config, backendFor });
