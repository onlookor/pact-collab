import { mkdirSync, appendFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Append-only JSONL sink for the audit mirror.
 * The directory and file are created with owner-only permissions.
 */
export class FileSink {
  constructor(dir, file = 'events.jsonl') {
    this.dir = dir;
    this.file = join(dir, file);
    mkdirSync(dir, { recursive: true, mode: 0o700 });
  }

  append(line) {
    appendFileSync(this.file, line.endsWith('\n') ? line : `${line}\n`, { mode: 0o600 });
  }

  close() {
    // appendFileSync opens and closes per call; nothing to release.
  }
}

const AUDIT_FIELDS = ['type', 'seq'];

function pick(value) {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

/**
 * Reduce a DSH session event to the audit fields we are allowed to persist.
 * Tool arguments and results are deliberately excluded.
 */
export function auditRecord(event) {
  const record = {};
  for (const field of AUDIT_FIELDS) {
    if (event?.[field] !== undefined) record[field] = event[field];
  }
  const tool = pick(event?.tool) ?? pick(event?.data?.name) ?? pick(event?.data?.tool);
  const callId = pick(event?.callId) ?? pick(event?.data?.callId) ?? pick(event?.data?.id);
  if (tool !== undefined) record.tool = tool;
  if (callId !== undefined) record.callId = callId;
  return record;
}

/**
 * Bounded in-memory audit buffer plus an optional sink.
 * Sink failures never propagate into the harness.
 */
export function createObserver({ limit = 256, sink = null, onError = null } = {}) {
  if (!Number.isInteger(limit) || limit < 1) throw new TypeError('limit must be a positive integer');
  const records = [];

  return {
    record(event) {
      const entry = auditRecord(event);
      if (Object.keys(entry).length === 0) return;
      records.push(entry);
      while (records.length > limit) records.shift();
      if (sink !== null) {
        try {
          sink.append(JSON.stringify(entry));
        } catch (error) {
          onError?.(error);
        }
      }
    },
    snapshot() {
      return records.slice();
    },
    close() {
      sink?.close?.();
    },
  };
}
