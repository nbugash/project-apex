<script lang="ts">
  /// Three versions and a choice (US4, FR-021, FR-033). Not in the prototype: recorded as a
  /// deviation in spec.md, and built from design-system tokens only so a designer can move it.
  ///
  /// The client decides nothing here. The editable result starts as the merge's draft -- markers
  /// around the colliding regions only (scenario 7) -- and the core refuses it while markers remain,
  /// so the only things ever written are a result the developer confirmed or one side, whole.
  import {
    REASON,
    SIDES,
    sideContent,
    wholeFileChoices,
    type Conflict,
  } from './conflict-presentation';
  import type { Resolution, ResolveOutcome } from './conflicts.svelte';

  interface Props {
    conflicts: readonly Conflict[];
    onresolve: (c: Conflict, r: Resolution) => Promise<ResolveOutcome>;
  }
  let { conflicts, onresolve }: Props = $props();

  let selectedPath = $state<string | null>(null);
  let current = $derived(
    conflicts.find((c) => c.relativePath === selectedPath) ?? conflicts[0] ?? null,
  );
  /// The developer's edits, per file and per remote version: a newer remote after a stale refusal
  /// brings a new draft rather than keeping an edit made against the old one.
  let edits = $state<Record<string, string>>({});
  let message = $state<string | null>(null);
  let busy = $state(false);

  const key = (c: Conflict) => `${c.relativePath}@${c.remoteSha256 ?? 'deleted'}`;
  let result = $derived(current ? (edits[key(current)] ?? current.draft ?? '') : '');
  let choices = $derived(current ? wholeFileChoices(current) : null);

  async function choose(r: Resolution) {
    if (!current || busy) return;
    busy = true;
    const outcome = await onresolve(current, r);
    busy = false;
    message = outcome.outcome === 'resolved' ? null : outcome.message;
  }
</script>

{#if current && choices}
  <section class="conflicts" data-testid="conflict-panel" aria-labelledby="conflicts-title">
    <header>
      <i class="ph ph-git-merge" aria-hidden="true"></i>
      <h2 id="conflicts-title">
        {conflicts.length === 1 ? '1 file needs' : `${conflicts.length} files need`} your decision
      </h2>
    </header>

    {#if conflicts.length > 1}
      <ul class="files" aria-label="Files in conflict">
        {#each conflicts as c (c.relativePath)}
          <li>
            <button
              type="button"
              data-testid="conflict-row"
              aria-current={c.relativePath === current.relativePath}
              onclick={() => {
                selectedPath = c.relativePath;
                message = null;
              }}>{c.relativePath}</button
            >
          </li>
        {/each}
      </ul>
    {/if}

    <p class="file" data-testid="conflict-file">{current.relativePath}</p>
    <p class="reason">{REASON[current.reason]}</p>

    <div class="sides">
      {#each SIDES as s (s.side)}
        {@const content = sideContent(current, s.side)}
        <section class="side" data-testid="conflict-side" data-side={s.side} aria-label={s.label}>
          <h3><i class="ph {s.icon}" aria-hidden="true"></i> {s.label}</h3>
          {#if 'text' in content}
            <pre>{content.text}</pre>
          {:else}
            <p class="absent">{content.absent}</p>
          {/if}
        </section>
      {/each}
    </div>

    {#if current.mergeable && current.draft !== null}
      <label class="result">
        <span>Result to write: settle each marked region, then use it</span>
        <textarea
          data-testid="conflict-result"
          spellcheck="false"
          value={result}
          oninput={(e) => (edits[key(current!)] = e.currentTarget.value)}
        ></textarea>
      </label>
    {/if}

    <div class="actions">
      {#if current.mergeable && current.draft !== null}
        <button
          type="button"
          data-testid="conflict-use-result"
          disabled={busy}
          onclick={() => choose({ kind: 'text', text: result })}>Use this result</button
        >
      {/if}
      <button
        type="button"
        data-testid="conflict-keep-local"
        disabled={busy}
        onclick={() => choose({ kind: 'keepLocal' })}>{choices.keepLocal}</button
      >
      <button
        type="button"
        data-testid="conflict-take-remote"
        disabled={busy}
        onclick={() => choose({ kind: 'takeRemote' })}>{choices.takeRemote}</button
      >
    </div>

    {#if message}
      <p class="message" role="status" data-testid="conflict-message">
        <i class="ph ph-info" aria-hidden="true"></i>
        {message}
      </p>
    {/if}
  </section>
{/if}

<style>
  /* Accent and neutral ramps and the divider only: the design system has no semantic tone token,
     and a conflict panel's job is to show three versions, not to alarm (T058). */
  .conflicts {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    max-block-size: 60%;
    overflow: auto;
    flex: 0 0 auto;
    color: var(--color-neutral-300);
    background: var(--color-surface);
    border-block-end: 1px solid var(--color-divider);
    font-family: var(--font-body);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-accent);
  }
  h2 {
    margin: 0;
    font-family: var(--font-heading);
    font-weight: var(--font-heading-weight);
    font-size: inherit;
    color: var(--color-neutral-100);
  }
  .files {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .file {
    margin: 0;
    font-family: var(--vk-mono);
    color: var(--color-neutral-100);
  }
  .reason {
    margin: 0;
  }
  .sides {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-2);
  }
  .side {
    min-inline-size: 0;
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    padding: var(--space-1) var(--space-2);
    font-size: inherit;
    font-weight: var(--font-heading-weight);
    color: var(--color-neutral-100);
    border-block-end: 1px solid var(--color-divider);
  }
  pre,
  textarea {
    margin: 0;
    padding: var(--space-2);
    font-family: var(--vk-mono);
    color: var(--color-neutral-100);
    white-space: pre;
    overflow: auto;
  }
  pre {
    max-block-size: 12rem;
  }
  .absent {
    margin: 0;
    padding: var(--space-2);
    font-style: italic;
    color: var(--color-neutral-400);
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  textarea {
    min-block-size: 8rem;
    resize: vertical;
    background: var(--color-bg);
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  button {
    padding: var(--space-1) var(--space-2);
    font: inherit;
    color: var(--color-neutral-100);
    background: var(--color-neutral-800);
    border: 1px solid var(--color-divider);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  button[aria-current='true'] {
    border-color: var(--color-accent);
  }
  button:disabled {
    cursor: progress;
    color: var(--color-neutral-400);
  }
  button:focus-visible,
  textarea:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
  .message {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    color: var(--color-neutral-100);
  }
</style>
