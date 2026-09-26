/// Routing an engine frame to the right panel.
///
/// `terminal-live.spec.ts` proves the whole path works and takes thirty seconds of real window to
/// do it, which makes it a poor place to assert what happens to a frame that is malformed, names
/// no task, or belongs to another feature. Those are decisions about a string, and they belong
/// where a string is cheap.
///
/// The listener itself is not exercised here: `listen` needs a Tauri host. What is exercised is
/// the routing the listener performs, extracted so it can be — which is the same reason
/// `applyChunk` and `summarise` are separate from the panel that calls them.
import { describe, expect, it } from 'vitest';
import { routeNotification } from '../../client/ui/lib/terminal/engine';
import { terminals } from '../../client/ui/lib/terminal/terminals.svelte';
import { encodeBase64 } from '../../client/ui/lib/terminal/wire';

function frame(method: string, params: unknown): { method: string; body: string } {
  return { method, body: JSON.stringify({ jsonrpc: '2.0', method, params }) };
}

const bytes = (text: string) =>
  encodeBase64(new Uint8Array([...text].map((c) => c.charCodeAt(0))));

describe('routing an engine notification', () => {
  it('writes output to the panel the frame names', () => {
    const f = frame('execution/onStdout', { task_id: 'routed', data: bytes('hello') });
    routeNotification(f.method, f.body);

    expect(terminals.has('routed')).toBe(true);
    // Read back through the panel's own accessor rather than a private field, so this asserts
    // what a renderer would receive.
    const held = new TextDecoder().decode(terminals.panel('routed').buffered());
    expect(held).toContain('hello');
  });

  it('creates a panel for a task it has never seen', () => {
    // A reattached task produces bytes before anything on screen has asked for it, so a route
    // that only delivered to existing panels would lose the first output of every recovery.
    expect(terminals.has('unseen')).toBe(false);
    const f = frame('execution/onStderr', { task_id: 'unseen', data: bytes('x') });
    routeNotification(f.method, f.body);
    expect(terminals.has('unseen')).toBe(true);
  });

  it('records an ending, keeping the signal rather than inventing a code', () => {
    const f = frame('execution/onExit', { task_id: 'killed', exit_code: null, signal: 'SIGTERM' });
    routeNotification(f.method, f.body);

    const ending = terminals.panel('killed').ending;
    expect(ending).toEqual({ exitCode: null, signal: 'SIGTERM' });
    // Specifically not 143. Manufacturing `128 + n` would rebuild one layer up the convention
    // the protocol spent a field preserving (FR-029).
    expect(ending?.exitCode).not.toBe(143);
  });

  it('ignores a method belonging to another feature', () => {
    const before = terminals.panels.length;
    const f = frame('workspace/onFileEvent', { task_id: 'not-a-task', data: bytes('x') });
    routeNotification(f.method, f.body);
    expect(terminals.panels.length).toBe(before);
  });

  it('drops a frame that will not parse rather than guessing at it', () => {
    const before = terminals.panels.length;
    routeNotification('execution/onStdout', '{not json');
    expect(terminals.panels.length).toBe(before);
  });

  it('drops a frame that names no task', () => {
    const before = terminals.panels.length;
    const f = frame('execution/onStdout', { data: bytes('orphan') });
    routeNotification(f.method, f.body);
    expect(terminals.panels.length).toBe(before);
  });
});
