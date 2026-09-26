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
  import { revealTerminal } from '../terminal/start';
  import { describeEnding } from '../terminal/ending';
  import EditorPanel from '../editor/EditorPanel.svelte';
  import DockTabs from '../chrome/DockTabs.svelte';
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
  /// The session's autosave preference, held here because the editor is remounted per tab and a
  /// component that read it itself would re-read it on every tab switch (FR-007b).
  let autosave = $state(session.autosave);
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

  function setRegionVisible(region: 'output', visible: boolean) {
    if (layout[region].visible === visible) return;
    layout = { ...layout, [region]: { ...layout[region], visible } };
    void persist(() => ipc.layoutSetRegion(region, visible, layout[region].extent));
  }

  /// How the terminal on screen ended, or null while it is running.
  ///
  /// Read from the panel the dock is showing rather than from the task set, because the tab
  /// describes what is in front of the developer. A second task ending elsewhere is that task's
  /// news, and overwriting the badge with it would tell them about a terminal they are not
  /// looking at.
  const terminalBadge = $derived.by(() => {
    const id = terminals.active;
    if (!id || !terminals.has(id)) return null;
    const ending = terminals.panel(id).ending;
    return ending ? describeEnding(ending) : null;
  });

  /// Which dock tab is selected, or `''` for none.
  ///
  /// Presentation, like `terminals.active`, so it lives here rather than in the session: the core
  /// owns what a task *is*, not which of four tabs is on top.
  ///
  /// **Empty on launch, and not persisted, which is what makes the click meaningful.** The dock
  /// itself defaults to visible (F000's layout), so a tab selected up front would mean a login
  /// shell running before anyone asked -- and a login shell runs the developer's whole profile.
  /// VS Code reaches the same behaviour by defaulting its panel closed; the dock here is open, so
  /// the distinction moves to the tab. Until one is clicked the panel shows its idle prompt,
  /// which is the prototype's own empty state.
  let dockTab = $state('');

  /// Clicking a dock tab, which is the only way the terminal starts.
  ///
  /// VS Code and IntelliJ both behave this way and the reason is worth stating: a login shell
  /// runs the developer's profile, and running it for somebody who never opened the panel is a
  /// side effect nobody asked for. Clicking the tab a second time shows the shell already
  /// running, because `revealTerminal` starts at most one.
  ///
  /// Clicking the current tab while the dock is open closes it, which is what both editors do
  /// and what makes the strip a control rather than a label.
  function selectDockTab(id: string) {
    if (dockTab === id && layout.output.visible) {
      setRegionVisible('output', false);
      return;
    }
    dockTab = id;
    setRegionVisible('output', true);
    if (id === 'terminal') void revealTerminal();
  }

  async function reload() {
    const s = await ipc.sessionGet();
    documents = s.documents;
    focusedId = s.focused_document_id;
    autosave = s.autosave;
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
        <FileTree
          tree={workspaceTree}
          selected={activeDocument?.path ?? ''}
          onOpenFile={(path, name) => persist(() => ipc.documentsOpen(name, path)).then(reload)}
        />
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
          <!-- Keyed on the document so switching tabs gives Monaco a fresh mount rather than a
               model swapped underneath it. The buffer behind it is not remounted: it lives in
               `buffers.svelte.ts` precisely so a tab switch cannot lose it. -->
          {#key activeDocument.id}
            <EditorPanel path={activeDocument.path} {autosave} />
          {/key}
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
      <!-- Below the dock, and outside the `visible` branch: the strip is how the dock is
           opened, so it cannot be inside the thing it opens. The prototype puts it here too. -->
      <DockTabs
        active={dockTab}
        open={layout.output.visible}
        onselect={selectDockTab}
        {terminalBadge}
      />
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
