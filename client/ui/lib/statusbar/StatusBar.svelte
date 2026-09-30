<script lang="ts">
  import type { ConnectionState, WorkspaceReference } from '../ipc';
  import { present } from './presentation';
  import { branchLabel } from '../git/branch';
  import type { GitBranch } from '../git/status.svelte';
  import {
    describeReconciliation,
    needsAttention,
    type Reconciliation,
  } from '../offline/reconciliation';

  /// Whether changes on the host are reaching the developer.
  ///
  /// Separate from `connection` because exhausted watch capacity happens while perfectly
  /// connected: a single connection state cannot express "the link is fine and you are not
  /// being told about changes", and FR-005 and FR-025 both require saying so.
  export type Reporting =
    | { kind: 'live' }
    | { kind: 'offline' }
    | { kind: 'partial'; unwatched: number }
    | { kind: 'unavailable' };

  interface Props {
    connection: ConnectionState;
    workspace: WorkspaceReference | null;
    persistenceFailed?: boolean;
    reporting?: Reporting;
    /// Where the repository is, or absent when there is no repository (F011, FR-018).
    ///
    /// **This surface is not in the prototype.** It is recorded as a deviation in spec.md, and
    /// everything about it comes from design-system tokens so a designer can move it without
    /// unpicking an improvised value (Principle I).
    branch?: GitBranch | null;
    /// How many files carry work the host has not seen (F012, FR-004).
    ///
    /// The connection half of "offline" is already on this bar: `PRESENTATION.disconnected`
    /// renders "Offline" with an icon, from F003. What this adds is the other half of §11.2 --
    /// that a file whose work is held locally is *shown* to be, so "saved" and "saved to the
    /// host" are never confused. A count rather than a list, because the list belongs to the
    /// tree and the editor; the bar's job is to say the work exists.
    ///
    /// Also not in the prototype, and recorded as the same deviation.
    heldLocally?: number;
    /// What the last reconciliation did (F012, FR-024): a summary here, every file in its title
    /// and accessible name. On the bar because the bar is where "held locally" was said, so it is
    /// where the developer looks to learn what became of it. Same deviation as above.
    reconciliation?: Reconciliation | null;
    ondismissreconciliation?: () => void;
  }
  let {
    connection,
    workspace,
    persistenceFailed = false,
    reporting = { kind: 'live' },
    branch = null,
    heldLocally = 0,
    reconciliation = null,
    ondismissreconciliation,
  }: Props = $props();

  let reconciled = $derived(reconciliation ? describeReconciliation(reconciliation) : null);

  let vcs = $derived(branchLabel(branch));

  /// What to say when changes are not being reported. Silence is how a developer would
  /// otherwise discover that watching failed, which is the outcome FR-005 forbids.
  function reportingNote(r: Reporting): string | null {
    switch (r.kind) {
      case 'live':
        return null;
      case 'offline':
        return 'Not watching for changes — showing what was last read';
      case 'partial':
        return `Not watching ${r.unwatched} folder${r.unwatched === 1 ? '' : 's'} — changes there will not appear`;
      case 'unavailable':
        return 'Not watching for changes — browsing and reading continue';
    }
  }

  let note = $derived(reportingNote(reporting));

  let state = $derived(present(connection));
</script>

<footer class="status" aria-label="Session status">
  {#if note}
    <!-- An icon and a label, so the state is not carried by colour alone (FR-039). -->
    <span class="reporting" data-testid="status-reporting" data-kind={reporting.kind}>
      <i class="ph ph-eye-slash" aria-hidden="true"></i>
      <span>{note}</span>
    </span>
  {/if}
  <span class="workspace" title={workspace?.name ?? 'No workspace'}>
    <i
      class="ph {workspace?.location_type === 'LOCAL' ? 'ph-desktop' : 'ph-cloud'}"
      aria-hidden="true"
    ></i>
    <span class="truncate">{workspace?.name ?? 'No workspace'}</span>
  </span>

  {#if vcs}
    <!-- Absent rather than empty when there is no repository: a blank slot reads as a
         rendering fault, and a placeholder makes a statement about git to a developer who is
         not using it (FR-019). -->
    <span class="branch" data-testid="status-branch" data-kind={branch?.kind} title={vcs.title}>
      <i class="ph {vcs.icon}" aria-hidden="true"></i>
      <span class="truncate">{vcs.text}</span>
    </span>
  {/if}

  <span class="connection" role="status" aria-live="polite">
    <i class="ph {state.icon}" aria-hidden="true"></i>
    <span>{state.label}</span>
  </span>

  {#if heldLocally > 0}
    <!-- FR-004: an icon and a word, so the state is never held in colour alone and survives a
         greyscale display -- the rule `presentation.ts` states for every connection state. -->
    <span class="held" role="status" aria-live="polite">
      <i class="ph ph-hard-drives" aria-hidden="true"></i>
      <span>{heldLocally} held locally</span>
    </span>
  {/if}

  {#if reconciliation && reconciled}
    <!-- A button because clicking it dismisses the report; the per-file detail is its title and
         accessible name, so it is reachable without a pointer. The icon differs by whether
         anything needs attention, so that is not carried by colour alone (FR-039). -->
    <button
      type="button"
      class="reconciled"
      data-testid="status-reconciled"
      data-attention={needsAttention(reconciliation)}
      title={`${reconciled.detail}\n\nClick to dismiss`}
      aria-label={`Reconciliation: ${reconciled.summary}. ${reconciled.detail.replaceAll('\n', '; ')}. Dismiss`}
      onclick={() => ondismissreconciliation?.()}
    >
      <i
        class="ph {needsAttention(reconciliation) ? 'ph-warning' : 'ph-check-circle'}"
        aria-hidden="true"
      ></i>
      <span>{reconciled.summary}</span>
    </button>
  {/if}

  {#if persistenceFailed}
    <!-- FR-023: reported, never modal, never blocking the interaction that triggered it. -->
    <span class="warn" role="status"
      ><i class="ph ph-warning" aria-hidden="true"></i> Not saved</span
    >
  {/if}
</footer>

<style>
  .status {
    display: flex;
    align-items: center;
    /* The prototype's own metrics, extracted by ds:sync. F000 built this bar from the
       generic spacing scale, which put it at 17px against the prototype's 26px — a
       difference nobody spotted by eye, but one that shortened the activity rail and the
       tool window by nine pixels each and so failed the fidelity gate. */
    gap: var(--vk-status-gap);
    padding: var(--vk-status-pad);
    background: var(--color-surface);
    box-shadow: inset 0 1px 0 color-mix(in srgb, var(--color-text) 9%, transparent);
    color: color-mix(in srgb, var(--color-text) 58%, transparent);
    font-family: var(--font-body);
    font-size: var(--vk-status-size);
    /* Fixed height so overlong content cannot displace adjacent content (FR-013). */
    block-size: var(--vk-status-height);
    white-space: nowrap;
    flex: 0 0 auto;
    overflow: hidden;
  }
  .workspace,
  .branch,
  .connection,
  .warn,
  .held,
  .reconciled {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-inline-size: 0;
  }
  .workspace {
    /* FR-013: truncate for display; the stored value is untouched. */
    max-inline-size: 40%;
  }
  .truncate {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .branch {
    /* The same truncation rule as the workspace name beside it: a long branch name is common
       and must not push the connection state off the bar. */
    max-inline-size: 30%;
    color: var(--color-neutral-300);
  }
  .connection {
    margin-inline-start: auto;
  }
  .reconciled {
    /* A report of ordinary work, in the same neutral ramp as `.held` for the same reason. The
       button is reset to read as the bar's other items, not as a control drawn on top of it. */
    color: var(--color-neutral-300);
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    cursor: pointer;
  }
  .reconciled:focus-visible {
    /* The shell's focus ring, at no offset: the bar clips overflow, so the usual 2px would be cut. */
    outline: 2px solid var(--color-accent);
    outline-offset: 0;
  }
  .held {
    /* The neutral ramp, not the accent one. Work held locally is an ordinary state of this
       feature and not a warning: it is what saving offline is supposed to do, and colouring it
       like `.warn` would tell a developer something went wrong when nothing did. */
    color: var(--color-neutral-300);
  }
  .warn {
    color: var(--color-accent-300);
  }
</style>
