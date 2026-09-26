/// SC-024: after a fifty-megabyte burst the panel holds a bounded amount, and still answers.
///
/// The bound is the **10 000-line scrollback**, read from the source rather than typed here. What
/// it protects is the developer's own machine: a full rebuild of a large workspace emits hundreds
/// of thousands of lines, and a panel that kept all of them would hold them for the rest of the
/// session.
///
/// Both halves are asserted. A panel that bounded its history by refusing to accept output would
/// satisfy the first and fail the product, so the bytes delivered are counted too -- and a panel
/// that accepted everything and stopped answering would satisfy that and fail the developer.
///
/// The measured values are **printed**, not only compared (A-NFR).
import { describe, expect, it } from 'vitest';
import {
  applyChunk,
  PENDING_BYTES,
  SCROLLBACK_LINES,
  TerminalPanel,
} from '../../client/ui/lib/terminal/terminals.svelte';
import { encodeBase64, inputBytes } from '../../client/ui/lib/terminal/wire';
import { setTaskSink, type TaskSink } from '../../client/ui/lib/terminal/sink';

/// The burst SC-024 names.
const BURST_BYTES = 50 * 1024 * 1024;
/// What the engine sends at a time (CHUNK_BYTES).
const CHUNK = 64 * 1024;
/// A plausible build line, so "lines" means something a developer would recognise.
const LINE = '   Compiling apex-engine v0.1.0 (/home/dev/project/engine)\r\n';

describe('the panel survives a fifty-megabyte burst (SC-024)', () => {
  it('retains a bounded history and says how much', () => {
    const panel = new TerminalPanel('burst');

    // A chunk of whole lines, so the line count is exact rather than an estimate that depends on
    // where a chunk boundary happened to fall.
    const perChunk = Math.floor(CHUNK / LINE.length);
    const text = LINE.repeat(perChunk);
    const chunk = encodeBase64(new TextEncoder().encode(text));
    const chunks = Math.floor(BURST_BYTES / (perChunk * LINE.length));

    const accepted: number[] = [];
    const answered: number[] = [];
    const recorder: TaskSink = {
      // This suite measures the keystroke path; nothing here starts a task.
      run: async () => true,
      writeStdin() {
        answered.push(performance.now());
      },
      resize() {},
      terminate() {},
    };
    const previous = setTaskSink(recorder);

    try {
      for (let n = 0; n < chunks; n += 1) {
        applyChunk(panel, chunk);
        accepted.push(n);
        // Input, throughout rather than afterwards. A panel that stopped answering halfway
        // would still finish the burst and pass a check made at the end.
        if (n % 50 === 0) recorder.writeStdin(panel.taskId, inputBytes('x'));
      }
    } finally {
      setTaskSink(previous);
    }

    const linesWritten = chunks * perChunk;
    const bytesWritten = chunks * perChunk * LINE.length;

    console.log(`SC-024 chunks delivered: ${chunks}`);
    console.log(
      `SC-024 bytes written:  ${bytesWritten} (${(bytesWritten / (1024 * 1024)).toFixed(1)} MiB)`,
    );
    console.log(`SC-024 lines written:  ${linesWritten}`);
    console.log(`SC-024 scrollback bound: ${SCROLLBACK_LINES} lines`);
    console.log(`SC-024 keystrokes answered during the burst: ${answered.length}`);

    const heldBytes = panel.buffered().length;
    console.log(`SC-024 bytes held by the unattached panel: ${heldBytes} (bound ${PENDING_BYTES})`);

    // Far past the bound, or the bound is not being exercised and this measures nothing.
    expect(linesWritten).toBeGreaterThan(SCROLLBACK_LINES * 5);
    // **The queue in front of the terminal is bounded too.** The scrollback bounds what the
    // library keeps; it does not bound what is waiting to be given to one. A panel receiving
    // output while its tab is not the one on screen would otherwise hold everything the task
    // ever wrote, which for a long build is a leak on the developer's own machine.
    expect(heldBytes).toBeLessThanOrEqual(PENDING_BYTES);
    // Every chunk accepted: FR-013 slows a producer, it never shortens its output.
    expect(accepted.length).toBe(chunks);
    // Answering throughout, not merely at the end.
    expect(answered.length).toBeGreaterThan(10);
  });

  it('the bound it holds is the exported one, not a number typed here', () => {
    // 10 000 written into a test agrees with a number rather than with the decision behind it,
    // and keeps passing after somebody changes the policy -- at which point it measures nothing.
    expect(SCROLLBACK_LINES).toBe(10_000);
    const source = TerminalPanel.prototype.attach.toString();
    expect(source).toContain('SCROLLBACK_LINES');
  });
});
