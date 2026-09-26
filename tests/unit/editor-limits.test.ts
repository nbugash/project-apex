/// The numbers that exist twice, checked against each other.
///
/// `MAX_TEXT_FILE` and the chunk threshold are fixed once in plan.md and then written down in
/// both runtimes, because Rust and TypeScript cannot share a constant. Principle II does not
/// stop applying because a language boundary is in the way: the way to keep one source of truth
/// across it is to make disagreement fail, which is what this does.
///
/// It reads the Rust source rather than a generated header. A generator would be another thing
/// to run, and the failure it protects against — somebody changing one number and not the other
/// — is exactly the failure a generator nobody ran would let through.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { CHUNK_THRESHOLD, MAX_TEXT_FILE } from '../../client/ui/lib/editor/buffers.svelte';

/// Evaluate a Rust integer expression of the form `64 * 1024 * 1024`.
function rustConst(source: string, name: string): number {
  const m = new RegExp(`const ${name}: u\\d+ = ([0-9*+ _]+);`).exec(source);
  if (!m) throw new Error(`${name} is no longer declared the way this test reads it`);
  return m[1]!
    .replace(/_/g, '')
    .split('*')
    .map((p) => Number(p.trim()))
    .reduce((a, b) => a * b, 1);
}

describe('the numbers plan.md fixes', () => {
  it('agrees with the core on the largest file opened as text', () => {
    const source = readFileSync(
      join(process.cwd(), 'client/core/src/adapters/inbound/tauri_commands.rs'),
      'utf8',
    );
    expect(rustConst(source, 'MAX_TEXT_FILE')).toBe(MAX_TEXT_FILE);
  });

  it('agrees with the wire on the chunk threshold', () => {
    // The threshold is not a number this feature chose: it is the inline read limit, which
    // A-BULKSIZE fixed at 512 KiB because content travels base64 and a threshold at §4.1's cap
    // would encode past it. plan.md said 1 MiB until this was checked.
    const source = readFileSync(join(process.cwd(), 'protocol/src/wire.rs'), 'utf8');
    expect(rustConst(source, 'MAX_INLINE_READ')).toBe(CHUNK_THRESHOLD);
  });
});
