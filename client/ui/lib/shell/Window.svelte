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
  import { GitStatusStore } from '../git/status.svelte';
  import { OfflineStore } from '../offline/state.svelte';
  import PathSearch from '../workspace/PathSearch.svelte';
  import { WatchRequester, watchedPaths } from '../workspace/watched.svelte';
  import { revealTerminal } from '../terminal/start';
  import { describeEnding } from '../terminal/ending';
  import EditorPanel from '../editor/EditorPanel.svelte';
  import { onMount } from 'svelte';
  import {
    startFileEvents,
    onHostFileEvents,
    onWorkspaceInvalidated,
    type HostFileEvent,
  } from '../editor/events';
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
  // Starts with the seeding identity the end-to-end suite drives, and is rebound to the real
  // one as soon as a workspace is announced. It was *only* ever `'e2e'` before F006, which is
  // why the tree could show seeded content and nothing else: it asks for a listing by workspace
  // id, and no real id ever reached it.
  const workspaceTree = new WorkspaceTree('e2e');
  // Reachable for the end-to-end suite, which seeds the projection through the Rust side and
  // then needs the tree to read it. The seeding command it pairs with is
  // `#[cfg(debug_assertions)]`, so on a release build there is nothing to drive this with.
  (window as unknown as Record<string, unknown>).__APEX_TREE__ = workspaceTree;

  // A file changing on the host concerns whichever buffer holds it (FR-024). Started once for
  // the window, not per editor: the panel is unmounted whenever its tab is not focused, and a
  // background tab is exactly the case FR-024 is about.
  onMount(() => startFileEvents());

  /// A wholesale invalidation dims the tree and re-reads nothing (§10.4, FR-017, FR-026a).
  onMount(() => onWorkspaceInvalidated(() => workspaceTree.invalidateAll()));

  /// The tree's half of a host file event.
  ///
  /// `deliver` reaches open buffers only, and a buffer knows nothing about rows -- so without
  /// this a file created on the host stays invisible until something re-lists its folder, on a
  /// tree whose whole design is to re-list nothing (FR-014). Registered once for the window,
  /// like the two above.
  onMount(() =>
    onHostFileEvents((events: HostFileEvent[]) => {
      for (const e of events) {
        const path = e.relative_path;
        const isDirectory = e.kind === 'directory';
        switch (e.event) {
          case 'created':
            workspaceTree.applyEvent({
              kind: 'created',
              path,
              isDirectory,
              size: e.size ?? 0,
              modified: e.modified ?? 0,
            });
            break;
          case 'modified':
            workspaceTree.applyEvent({
              kind: 'modified',
              path,
              size: e.size ?? 0,
              modified: e.modified ?? 0,
            });
            break;
          case 'deleted':
            workspaceTree.applyEvent({ kind: 'deleted', path });
            break;
          case 'renamed':
            if (e.to_path) {
              workspaceTree.applyEvent({ kind: 'renamed', path, toPath: e.to_path });
            }
            break;
          default:
            // A kind this build does not know changes nothing. Guessing would insert a row
            // for a file that may not exist.
            break;
        }
      }
    }),
  );

  /// What the engine is asked to watch.
  ///
  /// **Built in F004 and connected here**, which is the whole of this change: `WatchRequester`,
  /// `watchedPaths` and `delta` were written, unit-tested and reachable from nothing, so the
  /// client asked the engine to watch no path at any time. The consequence was not a crash --
  /// the tree simply never heard about a change on the host, which is indistinguishable from a
  /// host where nothing changed.
  ///
  /// F011 depends on it: an ordinary save writes neither `HEAD` nor `index`, so the workspace's
  /// own file events are the only signal that a tracked file was edited (A-GITNUDGE, FR-002a).
  const watcher = new WatchRequester((change) => {
    void ipc
      .workspaceWatch(change.add, change.remove)
      .then((outcome) => recordWatch({ asked: change, outcome }))
      .catch((e: unknown) => {
        // A refused path is reported **in** the outcome; a thrown error means the request
        // itself did not land, which is a different fact and one that used to vanish here.
        recordWatch({ asked: change, error: String(e) });
      });
  });

  /// What each watch request asked for and what came back.
  ///
  /// Recorded for the suite, for the reason `tree.svelte.ts` records listings: F004's watch
  /// specs assert `toBeGreaterThanOrEqual(0)`, which passes for a client whose every watch
  /// request failed -- and did. A swallowed failure and a working watch look identical from
  /// the DOM.
  function recordWatch(entry: unknown): void {
    const w = window as unknown as { __apexWatch?: unknown[] };
    w.__apexWatch = w.__apexWatch ?? [];
    w.__apexWatch.push(entry);
  }

  /// Folder paths for expanded folders and **file** paths for open tabs, deliberately: the
  /// engine derives the directories. Resolving that here would make a folder holding an open
  /// file one path for two reasons, and collapsing it would stop reporting a file still open
  /// inside it (FR-003c, A-WATCHSCOPE).
  function declareWatched(expanded: string[]): void {
    // The root is always in the set. It is displayed from the moment a workspace opens --
    // `open()` lists it and the tree shows its children -- so it is watched on the same terms
    // as any folder the developer expanded. `expandedFolders()` cannot report it, because the
    // root is not a row.
    watcher.update(
      watchedPaths(['/', ...expanded], session.documents.map((d) => d.path)),
    );
  }

  /// The window's git state, subscribed once, for the same reason as the line above.
  ///
  /// The project panel is unmounted whenever another rail destination is selected, so a
  /// subscription owned by the tree would stop hearing about changes the moment the developer
  /// looked at something else -- and the marks would be silently stale on return (F011,
  /// FR-002).
  const git = new GitStatusStore();
  $effect(() => {
    const started = git.start();
    return () => {
      void started.then(() => git.stop()).catch(() => {});
    };
  });

  /// The offline projection, subscribed once for the window on the same terms and for the same
  /// reason as the git store above: the status bar and the editor both read it, and a
  /// subscription owned by either would stop reporting the moment the developer looked elsewhere.
  ///
  /// No `stop`: this store owns no listener of its own. It follows `shellState.connection`, which
  /// `main.ts` already maintains, so there is nothing to unsubscribe and nothing that could keep
  /// the window alive.
  const offline = new OfflineStore();
  offline.start();

  /// Bind the tree to whichever workspace is open, and fetch its root.
  ///
  /// One listing, on open, and none afterwards: `open()` asks for the root only, and a folder's
  /// children are fetched the first time it is expanded (FR-014, SC-001). Guarded on the id
  /// actually changing, because this effect re-runs whenever anything it reads does, and a
  /// listing per re-evaluation is the sort of thing SC-002 counts.
  $effect(() => {
    const id = shellState.workspace?.id;
    if (!id || id === workspaceTree.workspaceId) return;
    workspaceTree.workspaceId = id;
    // A restored session names a workspace the engine has probably never heard of -- it exits
    // when the last client goes and nothing is left to preserve. Re-registering is what makes
    // the rest of this effect mean anything; without it the watch below is refused and the
    // workspace is inert while looking entirely normal.
    void ipc.workspaceResume(id).catch(() => {
      // A resume that fails leaves the cached projection readable, which is the offline
      // behaviour the developer already understands.
    });
    // **Before the listing, not after it.** The root is watched because the workspace is open,
    // not because its contents have arrived, and asking afterwards leaves a window -- the
    // listing round trip plus this requester's debounce -- in which a change on the host is
    // seen by nothing. It is a window a developer hits by editing a file immediately after
    // opening a workspace, which is an ordinary thing to do.
    declareWatched([]);
    void workspaceTree.open();
  });
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
        <!-- Above the tree, because a filter that appears below what it filters reads as a
             footnote. F012 adds it so FR-007's "the developer runs a path search" has somewhere to
             happen; recorded as a Principle I deviation, like the conflict panel. -->
        <PathSearch
          onOpenFile={(path, name) => persist(() => ipc.documentsOpen(name, path)).then(reload)}
        />
        <FileTree
          tree={workspaceTree}
          selected={activeDocument?.path ?? ''}
          gitStatus={(path) => git.stateOf(path)}
          offline={!offline.connected}
          onWatchedChanged={declareWatched}
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
            <EditorPanel
              path={activeDocument.path}
              {autosave}
              gitRevision={git.revision}
              heldLocally={offline.isHeldLocally(activeDocument.path)}
            />
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
    branch={git.branch}
    heldLocally={offline.pending.length}
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
    /* A flex column, so a panel inside it can claim the height rather than being sized by its
       own content. The editor is the first child that needs this: as a plain block container
       this measured 5px, Monaco's automaticLayout observed that and rendered a single line, and
       everything below the first line silently stopped existing. */
    display: flex;
    flex-direction: column;
    min-block-size: 0;
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
