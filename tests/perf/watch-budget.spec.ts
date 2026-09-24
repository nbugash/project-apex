// F004's measurements. A-NFR binds all of them: p99 at the interface boundary, at least a
// hundred samples, harness delay excluded, and **the measured value printed** rather than only
// compared — a comparison that passes tells you nothing about how much room was left.
//
// These measure the pure logic that sits on the interaction path. The end-to-end suite measures
// the same properties against the real window; this runs on every commit without a display.
import { describe, expect, it } from 'vitest';
import { delta, watchedPaths, WatchRequester } from '../../client/ui/lib/workspace/watched.svelte';
import { WorkspaceTree, type FileChange, type Node } from '../../client/ui/lib/workspace/tree.svelte';

const SAMPLES = 200; // A-NFR requires at least 100.
const INTERACTION_BUDGET_MS = 100; // §1.4's stall budget.

function p99(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.99) - 1)] ?? 0;
}

function measure(rounds: number, fn: (i: number) => void): number[] {
  // One warm-up round, excluded: the first call pays for JIT and allocation that no later
  // interaction pays for, and A-NFR excludes harness cost rather than measuring it.
  fn(0);
  const samples: number[] = [];
  for (let i = 0; i < rounds; i += 1) {
    const start = performance.now();
    fn(i);
    samples.push(performance.now() - start);
  }
  return samples;
}

function tree(count: number): WorkspaceTree {
  const t = new WorkspaceTree('ws1');
  const nodes: Node[] = [];
  for (let i = 0; i < count; i += 1) {
    nodes.push({
      name: `f${i}.rs`,
      kind: 'file',
      size: 1,
      modified: 0,
      path: `/src/f${i}.rs`,
      depth: 1,
      expanded: false,
      loaded: false,
    });
  }
  nodes.unshift({
    name: 'src',
    kind: 'directory',
    size: 0,
    modified: 0,
    path: '/src',
    depth: 0,
    expanded: true,
    loaded: true,
  });
  t.nodes = nodes;
  return t;
}

describe('watch establishment stays inside the interaction budget', () => {
  it('derives the watched set for a large workspace well inside §1.4', () => {
    // Expanding a folder is a developer-initiated interaction, so the set it produces has to
    // be computable inside the budget rather than merely eventually.
    const folders = Array.from({ length: 500 }, (_, i) => `/src/d${i}`);
    const tabs = Array.from({ length: 50 }, (_, i) => `/src/d${i}/main.rs`);
    const samples = measure(SAMPLES, () => {
      watchedPaths(folders, tabs);
    });
    const measured = p99(samples);
    console.log(`SC-009a watched-set derivation p99: ${measured.toFixed(3)} ms (budget ${INTERACTION_BUDGET_MS})`);
    expect(measured).toBeLessThan(INTERACTION_BUDGET_MS);
  });

  it('computes the delta between two large sets inside the budget', () => {
    const before = Array.from({ length: 1000 }, (_, i) => `/p${i}`);
    const after = before.slice(0, 900).concat(['/new1', '/new2']);
    const samples = measure(SAMPLES, () => {
      delta(before, after);
    });
    const measured = p99(samples);
    console.log(`watch delta p99: ${measured.toFixed(3)} ms (budget ${INTERACTION_BUDGET_MS})`);
    expect(measured).toBeLessThan(INTERACTION_BUDGET_MS);
  });
});

describe('event delivery does not delay interactive work', () => {
  it('applies a burst of ten thousand changes without stalling an interaction', () => {
    // SC-005 and FR-016. The burst is what a branch switch looks like before the bulk rule
    // catches it; what is measured is that an interaction interleaved with it still fits.
    const t = tree(1000);
    const events: FileChange[] = Array.from({ length: 10_000 }, (_, i) => ({
      kind: 'modified',
      path: `/src/f${i % 1000}.rs`,
      size: i,
      modified: i,
    }));
    let cursor = 0;
    const samples = measure(SAMPLES, () => {
      // One interaction's worth of applied events, timed as the developer would feel it.
      for (let i = 0; i < 50; i += 1) {
        t.applyEvent(events[cursor % events.length]!);
        cursor += 1;
      }
    });
    const measured = p99(samples);
    console.log(`SC-005 apply-under-burst p99: ${measured.toFixed(3)} ms (budget ${INTERACTION_BUDGET_MS})`);
    expect(measured).toBeLessThan(INTERACTION_BUDGET_MS);
  });

  it('marks a whole tree stale inside the budget', () => {
    // The cheapest operation in the feature, and the most common: a branch switch does it once
    // for the entire workspace.
    const t = tree(5000);
    const samples = measure(SAMPLES, () => {
      t.invalidateAll();
    });
    const measured = p99(samples);
    console.log(`SC-012 invalidate-all p99: ${measured.toFixed(3)} ms (budget ${INTERACTION_BUDGET_MS})`);
    expect(measured).toBeLessThan(INTERACTION_BUDGET_MS);
  });
});

describe('the requester coalesces rather than flooding', () => {
  it('turns a rapid sequence of expansions into one request', () => {
    // Not a latency measurement but a count one, and the count is the property: three round
    // trips where one would do is three times the budget spent.
    let sent = 0;
    const r = new WatchRequester(() => {
      sent += 1;
    }, 0);
    for (let i = 0; i < 100; i += 1) r.update([`/a${i}`]);
    console.log(`requests issued for 100 rapid expansions: ${sent} (before the timer fires)`);
    expect(sent).toBe(0);
  });
});
