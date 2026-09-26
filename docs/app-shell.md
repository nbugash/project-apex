# Application Shell (F000)

What later features need to know about the shell they mount into. The specification lives in
`specs/001-app-shell/`; this is the working map.

## Shape

Two runtimes in one process. The Rust core owns state, persistence and the window; the
webview owns pixels. **The core owns truth and the webview owns pixels** is the invariant —
the webview holds no authoritative state and can be reloaded without loss.

```
client/core/src/
  domain/        no external imports at all
  application/   ports + use cases; depends only on domain and ports
  adapters/      inbound (Tauri commands), outbound (store, connection)
  composition.rs the only place adapters are bound
```

Dependencies point inward. Nothing in `domain` or `application` imports Tauri; if you find
yourself wanting to, the boundary is in the wrong place.

## Extending it

**Adding a command.** Add the signature to `contracts/shell-commands.md` first, then the
handler in `adapters/inbound/tauri_commands.rs`, then register it in `lib.rs`. Validate
arguments in the handler: input from the webview is untrusted regardless of what the
interface layer already checked (Principle VI).

**Adding a side effect.** It becomes an outbound port in `application/ports/`, named for the
capability rather than the technology — `SessionStore`, not `JsonFileStore`. Implement it in
`adapters/outbound/` and bind it in `composition.rs`.

**Replacing the connection source (F001).** `StubConnectionStatusSource` implements
`ConnectionStatusSource`. Swap the binding in `composition.rs`; nothing else changes. That
one-line swap is the property the port structure exists to buy.

## Reaching the engine

Until F010 the application never sent a request to an engine for any feature. The transport was
built and used only as a connection-status source; `RemoteWorkspaceProvider` and `RemoteTasks`
were both written, both tested against doubles, and neither could be constructed, because nothing
implemented the `RequestSender` port they depend on.

The chain, now closed:

```
webview  ──invoke──▶  task_commands.rs  ──▶  RemoteTasks  ──▶  TransportSender
                                                                     │
                                                              SshTransport
                                                                     │
                                                    ssh  ──or──  LocalEngineSpawner
                                                                     │
                                                                ide-engine
engine  ──frame──▶  NotificationSink  ──▶  WebviewNotifications  ──emit──▶  engine.ts  ──▶  panel
```

**`TransportSender` is concrete in `SshTransport` on purpose.** The generic version does not
compile: `RequestTransport::send` is a native `async fn` in a trait, so for an unknown `T` the
compiler cannot prove the returned future is `Send`, and a provider must be `dyn` **and** `Send`.
Naming the one implementation is what lets it check the future it is checking. `request_sender.rs`
exists for exactly this reason and is worth reading before generalising it.

**A workspace is registered with the engine when it is opened**, in `workspace_open`, because
that is the moment the root path is in hand. Nothing did this before, so the engine would have
refused every task with `-32001`. A task command does **not** accept a `workspaceId` from the
webview: the core registered the workspace, so the core knows which one, and an interface that
named it would be choosing where a command runs (Principle VI).

## What the store remembers (A-STATE, A-STATE2)

`PersistedSession` is at **schema version 3**. Each bump added fields, and each added them with
`#[serde(default)]` — that attribute *is* the migration. A file written by the previous version
has no such field, parses, and gets the default, which is what keeps an existing user's geometry,
layout and tabs across the upgrade instead of discarding the session because one key is missing.

Version 3 added **task identities** (A-STATE2, FR-031d): a `task_id` and the `workspace_id` that
owns it, per task this client started.

**Identities and nothing else, deliberately.** A task outlives the connection that started it, and
`execution/attach` reaches one only by an identity the caller already knows — so a client that
restarts needs its identities to have survived the restart, or reattachment works only for a
client that never closed.

Two things are excluded for reasons worth keeping:

- **No command.** A command line can carry a credential in argv, and FR-005a's accepted boundary
  does not extend to writing that to disk.
- **No output.** Retention belongs to the engine, which bounds it. A store that accumulated output
  would grow without limit for a client that never comes back.

The identity is the smallest thing that restores reachability. A client that has lost even that
is not stranded — `execution/list` enumerates what the engine still holds — but that is the
recovery path, and it costs a round trip on the one path where the developer is waiting and the
link has just proved unreliable. Remembering is what keeps it off the ordinary path.

## Things that will bite you

**The window is created hidden.** It becomes visible only when the interface calls
`shell_ready`. If you add startup work before that call, the window stays dark for longer;
if you remove the call, it never appears. This is deliberate — it makes "no unstyled frame"
(FR-020) structural rather than a race.

**Session state is not the workspace cache.** It lives in its own JSON file for reasons
recorded in Appendix A, A-STATE. Do not move it into the SQLite cache F003 introduces: that
cache is disposable and evicted, and interface state is not.

**Persistence is debounced.** Mutations return immediately and a writer thread coalesces
them. A failed write is reported, never propagated to the caller (FR-023). Do not add an
await on persistence to an interaction path.

**Styling comes from `mockups/`.** `npm run ds:sync` copies it; `client/ui/lib/ds/` is build output
and is gitignored. Never hand-edit a token value. `npm run lint:ds` fails on raw hex, raw px
beyond 1-2px hairlines, and hard-coded fonts; `ds:sync` fails on a reference to a token the
design system does not define, which is the FR-022 designer-gap signal.

**Known design gap.** The system defines `--font-heading` and `--font-body` (both Inter) but
no mono token, although JetBrains Mono is bundled. The editor feature will need one; that is
a designer conversation, not an implementation choice.

## Running it

```bash
npm install && npm run ds:sync
npm run tauri dev          # development
cargo test --manifest-path client/core/Cargo.toml
npm run test:unit
npm run lint:ds            # run before pushing
npm run perf:budget        # run before pushing
xvfb-run -a npm run e2e    # Linux only; see Appendix A, A-E2E
```

End-to-end runs on Linux only: macOS provides no WebDriver for its platform webview. macOS
gets `npm run smoke:macos`, which proves the app starts, paints and exits cleanly — it
exercises no user journey and is not a substitute.

`APEX_DATA_DIR` overrides the profile directory. The end-to-end suite uses it to give each
run an isolated profile it can seed and corrupt.

---

# Shell Chrome (F018)

The prototype's chrome, mounted on the F000 shell. Specification in
`specs/002-shell-chrome-fidelity/`.

## Components

```
client/ui/lib/chrome/
  ChromeHeader.svelte   product mark, project switcher, run group, omnibox, right cluster
  ActivityRail.svelte   the destination list, roving tab index, collapse toggle
  RailButton.svelte     one destination: active, available, unavailable
  ToolWindow.svelte     the panel frame and its header row
```

The rail's destinations come from the core (`rail_destinations`), not from the interface.
They are behaviour — identity, label, icon, availability — and the core is where behaviour
lives. `client/ui/lib/rail.ts` holds the ordering and keyboard helpers so they can be tested
without a window.

**Every control in the chrome header is inert.** Opening a workspace, run configurations,
the omnibox and settings all belong to features that do not exist. They render at full
fidelity and carry `aria-disabled` with no tab stop, so the header looks like the approved
design without claiming behaviour it does not have. Omitting them instead would change the
header's proportions on every release.

## Dimensions come from the prototype, never from you

No chrome dimension is written in a component. `scripts/ds-sync.mjs` extracts them from
`mockups/` on every build into `client/ui/lib/ds/layout-tokens.css`, and components reference
`var(--vk-*)`. `lint:ds` rejects a raw pixel value, and an end-to-end test
(`chrome-tokens.spec.ts`) rejects one that reaches the rendered stylesheet.

Two things about that extraction are worth knowing before you change it:

**The prototype has three density presets** — `compact`, `default`, `roomy` — applied from
script on mount. Its prop schema declares `roomy` as the default, and that is what the
signed-off screens show. The `var(--vk-tool, 276px)` fallbacks scattered through its markup
encode the `default` preset and are _never_ the values displayed. Reading them produced a
token file that disagreed with every approved screen; the extractor now resolves the preset
the way the prototype does.

**Anchors are structural.** Each surface is located once by something stable
(`data-screen-label="Chrome"`, `<sc-for list="{{ rail }}">`) and its dimensions read from
the style strings that follow. Matching on values instead would silently start matching a
different element that happens to share a number.

## Screenshots

Every end-to-end test writes one to `reports/screenshots/${os}/${title}-${timestamp}.png`.
Captured from X with ImageMagick rather than through WebDriver: `saveScreenshot` against
WebKitWebDriver times out, and a display capture includes the native window rather than only
the webview viewport.

`assertCaptureIsNotBlank` fails any test whose capture has fewer than 16 distinct colours.
F000 shipped a window that was never made visible — the interface waited for animation
frames that a hidden window never produces — and every assertion passed while every
screenshot was black. A person found it by opening an image. This is that check.

## The fidelity gate

```bash
npm run gate:fidelity          # compare; exit 1 on difference, 2 if it cannot run
npm run gate:fidelity:update   # approve a new baseline, deliberately
npm run gate:fidelity:test     # the gate's own tests
```

It makes two judgements, and the difference matters:

- **Geometry**, against numbers derived from the **prototype**. This is the fidelity check.
- **Pixels**, against a baseline image of **this application**, approved once its geometry
  was reconciled against the prototype. This is the regression check.

A pixel baseline taken from the prototype could never pass — it is a populated mock with a
file tree, an editor and a terminal — so it would be red from the first run and switched
off. A geometry baseline taken from our own build would enforce self-consistency and pass
even if we had never matched the design. Neither alone is the gate.

`tools/gate-fidelity/reference/derivation.md` records where the numbers came from and what
the first reconciliation found. Do not edit `geometry.json` to make the gate pass; the
numbers are the design's.

## Things that will bite you

**The rail and the document tabs both use `role="tab"`.** A bare `[role="tab"]` selector
matches eleven elements. Scope it: `nav.rail [role="tab"]` or `.strip [role="tab"]`.

**The rail keeps its active mark while the tool window is collapsed**, because the prototype
does. It is the only cue to what reopening will show.

**Launching takes 22 seconds on a machine with no desktop session**, waiting out a portal
activation timeout that can never be satisfied. The harness and the gate set
`DBUS_SESSION_BUS_ADDRESS=/dev/null`, which brings it to about one second. If a suite
suddenly takes twenty times longer, that variable is the first thing to check.

**A debug build loads `devUrl`.** Without a server on port 1420 the webview is blank and
every selector times out with no clue why. The harness and the gate both start
`vite preview` for this reason.

**The status bar's height is load-bearing.** The rail and tool window occupy whatever the
chrome header and status bar leave. F000 built the status bar from the generic spacing
scale, which made it 17px against the prototype's 26px, and both gated surfaces came out
nine pixels too tall. Its metrics are extracted from the prototype now.
