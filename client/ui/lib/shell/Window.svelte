<script lang="ts">
  import Region from './Region.svelte';
  import Splitter from './Splitter.svelte';
  import ChromeHeader from '../chrome/ChromeHeader.svelte';
  import ActivityRail from '../chrome/ActivityRail.svelte';
  import ToolWindow from '../chrome/ToolWindow.svelte';
  import FileTree from '../workspace/FileTree.svelte';
  import { WorkspaceTree } from '../workspace/tree.svelte';
  import { MIN_TOOL_WINDOW_WIDTH } from '../rail';
  import TabStrip from '../tabs/TabStrip.svelte';
  import StatusBar from '../statusbar/StatusBar.svelte';
  import TerminalPanel from '../terminal/TerminalPanel.svelte';
  import { terminals } from '../terminal/terminals.svelte';
  import { installTerminalHarness } from '../terminal/harness';
  import { listenToEngine } from '../terminal/engine';
  import * as ipc from '../ipc';
  import type { SessionSnapshot } from '../ipc';
  import { shellState } from '../state.svelte';

  // The end-to-end suite's way in to the terminal renderer. Installs nothing outside
  // automation; `harness.ts` says why that guard is not merely tidiness.
  $effect(() => installTerminalHarness());

  // The product route for a task's output. Separate from the harness above, which exists only
  // under automation: this one runs always, and both end at `applyChunk` so the suite drives the
  // same rendering the engine does.
  $effect(() => {
    // The bridge is async, so the effect can be torn down before the listener is registered.
    // Awaiting the promise before unlistening is what stops a listener outliving this component.
    const pending = listenToEngine();
    return () => {
      void pending.then((unlisten) => unlisten()).catch(() => {});
    };
  });

  /// `Terminal — <workspace>` remotely, `Terminal — local` locally, following the prototype.
  const dockTitle = $derived.by(() => {
    const ws = shellState.workspace;
    if (!ws) return 'Terminal';
    return ws.location_type === 'REMOTE' ? `Terminal \u2014 ${ws.name}` : 'Terminal \u2014 local';
  });

  interface Props {
    session: SessionSnapshot;
  }
  let { session }: Props = $props();

  // Mirrors the core's MIN_REGION_EXTENT. The core rejects anything below it on a live
  // command, so clamping here keeps a drag from generating rejected round trips.
  const MIN_REGION_EXTENT = 120;

  let layout = $state(session.layout);
  let documents = $state(session.documents);
  let toolWindow = $state(session.tool_window);
  let destinations = $state<ipc.RailDestination[]>([]);

  // The catalogue is static for the process lifetime, so it is fetched once rather than
  // recomputed per render.
  $effect(() => {
    void ipc
      .railDestinations()
      .then((d) => (destinations = d))
      .catch((e) => console.warn('could not load rail destinations', e));
  });

  let activeDestination = $derived(
    destinations.find((d) => d.id === toolWindow.active_destination_id) ?? null,
  );

  async function selectDestination(id: string) {
    // Render from the core's answer rather than guessing: selecting the active destination
    // collapses it (FR-006), so the resulting state is not a function of the click alone.
    try {
      toolWindow = await ipc.railSelect(id);
    } catch (e) {
      persistenceFailed = true;
      console.warn('rail selection failed', e);
    }
  }

  function toggleToolWindow() {
    const active = toolWindow.active_destination_id;
    if (active) void selectDestination(active);
  }

  function resizeToolWindow(width: number) {
    toolWindow = { ...toolWindow, width };
    void persist(() => ipc.toolWindowResize(width));
  }
  // One tree for the window. Its workspace id is empty until one is opened, and the store
  // surfaces the resulting refusal rather than rendering an empty panel that looks like an
  // empty repository.
  const workspaceTree = new WorkspaceTree('e2e');
  // Reachable for the end-to-end suite, which seeds the projection through the Rust side and
  // then needs the tree to read it. The seeding command it pairs with is
  // `#[cfg(debug_assertions)]`, so on a release build there is nothing to drive this with.
  (window as unknown as Record<string, unknown>).__APEX_TREE__ = workspaceTree;
  let focusedId = $state(session.focused_document_id);
  let persistenceFailed = $state(false);

  async function persist(fn: () => Promise<unknown>) {
    try {
      await fn();
      persistenceFailed = false;
    } catch (e) {
      // FR-023: surface it, never block or reverse the interaction that caused it.
      persistenceFailed = true;
      console.warn('shell command failed', e);
    }
  }

  function resizeRegion(region: ipc.RegionId, extent: number) {
    // Render from local state immediately; the command is fire-and-forget so disk latency
    // never enters the gesture (SC-004).
    layout = { ...layout, [region]: { ...layout[region], extent } };
    void persist(() => ipc.layoutSetRegion(region, layout[region].visible, extent));
  }

  function toggleRegion(region: 'output') {
    const visible = !layout[region].visible;
    layout = { ...layout, [region]: { ...layout[region], visible } };
    void persist(() => ipc.layoutSetRegion(region, visible, layout[region].extent));
  }

  async function reload() {
    const s = await ipc.sessionGet();
    documents = s.documents;
    focusedId = s.focused_document_id;
  }

  let activeDocument = $derived(documents.find((d) => d.id === focusedId) ?? null);
</script>

<div class="shell">
  <ChromeHeader workspace={shellState.workspace?.name ?? null} />

  <div class="body">
    <ActivityRail
      {destinations}
      activeId={toolWindow.active_destination_id}
      collapsed={toolWindow.collapsed}
      onselect={selectDestination}
      ontoggle={toggleToolWindow}
    />

    <ToolWindow
      title={activeDestination?.label ?? 'Project'}
      width={toolWindow.width}
      collapsed={toolWindow.collapsed}
      ontoggle={toggleToolWindow}
    >
      <!-- The frame was F001's deliverable; filling it belongs to the feature behind each
           destination. F003 fills `project` with the file tree (FR-014, §10.1). Every other
           destination still shows the placeholder, and the wording matters: an available
           destination whose panel said it was "not available" contradicted the rail, which
           shows it as open and active. -->
      {#if activeDestination?.id === 'project'}
        <FileTree tree={workspaceTree} />
      {:else}
        <p class="pending">No workspace open.</p>
      {/if}
    </ToolWindow>

    {#if !toolWindow.collapsed}
      <Splitter
        orientation="vertical"
        extent={toolWindow.width}
        min={MIN_TOOL_WINDOW_WIDTH}
        label="Resize tool window"
        onresize={(e) => resizeToolWindow(e)}
      />
    {/if}

    <main class="document-area" aria-label="Documents">
      <TabStrip
        {documents}
        {focusedId}
        onfocus={(id) => persist(() => ipc.documentsFocus(id)).then(reload)}
        onclose={(id) => persist(() => ipc.documentsClose(id)).then(reload)}
        onreorder={(id, to) => persist(() => ipc.documentsReorder(id, to)).then(reload)}
      />
      <div class="content">
        {#if activeDocument}
          <p>{activeDocument.display_name}</p>
        {:else}
          <p class="empty">No document open</p>
        {/if}
      </div>

      {#if layout.output.visible}
        <Splitter
          orientation="horizontal"
          extent={layout.output.extent}
          min={MIN_REGION_EXTENT}
          label="Resize output"
          onresize={(e) => resizeRegion('output', e)}
        />
        <Region
          label={dockTitle}
          testid="region-output"
          visible={layout.output.visible}
          extent={layout.output.extent}
          axis="block"
        >
          <TerminalPanel taskId={terminals.active} workspace={shellState.workspace} />
        </Region>
      {/if}
    </main>
  </div>

  <StatusBar
    connection={shellState.connection}
    workspace={shellState.workspace}
    {persistenceFailed}
  />
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    block-size: 100vh;
    background: var(--color-bg);
    color: var(--color-text);
    font-family: var(--font-body);
  }
  .body {
    display: flex;
    flex: 1 1 auto;
    min-block-size: 0;
  }
  .document-area {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-inline-size: 0;
    min-block-size: 0;
  }
  .content {
    flex: 1 1 auto;
    padding: var(--space-4);
    overflow: auto;
  }
  .empty {
    color: var(--color-neutral-400);
  }
  .pending {
    margin: 0;
    padding: var(--space-3);
    color: var(--color-neutral-400);
    font-size: var(--vk-fs);
  }
</style>
