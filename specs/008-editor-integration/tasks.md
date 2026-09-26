# Tasks: Editor Integration

**Feature**: F006 `editor-integration` | **Branch**: `feature/F006-editor-integration`

**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/write-file.md](./contracts/write-file.md),
[design.md](./design.md), [architecture.md](./architecture.md), [quickstart.md](./quickstart.md)

Tests are **not optional here**: Constitution Principle VII requires unit, integration and
end-to-end coverage wherever the feature has surface, and this one has surface at all three.
Where a wrong answer is expensive — the base-hash comparison, path containment, range stitching
— the test is written first, which the same principle prefers and this project has twice been
glad of.

---

## Phase 1: Setup

- [X] T001 Add `monaco-editor` to `package.json`, and confirm it is the only dependency this feature adds. It goes in the webview bundle and nowhere else: Principle VIII forbids Monaco types in `client/core`, and a dependency that can only be imported from one directory is the cheapest way to keep that true
- [X] T002 **No ds-sync change needed, and that is the finding.** The prototype's editor colours are hex literals whose values are already design-system tokens: `#9397ab` is `--color-neutral-500`, `#e4e7f5` is `--color-neutral-200`, `#b5abfc` is `--color-accent-400`, `#d2cefd` is `--color-accent-300`, `#75798c` is `--color-neutral-600`. Extracting them would create five tokens duplicating five that exist, with nothing keeping the copies in step. A-EDITPALETTE is corrected to say so
- [X] T003 `tests/unit/editor-palette.test.ts`: assert each of the five hex literals in `mockups/Apex IDE (standalone).html`'s editor markup still equals the design-system token this feature maps it to, reading both from disk. **This is what keeps the mapping honest**: the prototype could change a colour and the application would keep rendering the old token, which is drift in the direction Principle I exists to catch, and no other check would see it

---

## Phase 2: Foundational — the editor surface exists

**Blocks every user story.** Until a buffer can be mounted and shown, none of the four stories
has anywhere to happen.

- [X] T004 [P] `client/ui/lib/editor/ranges.ts`: `LoadedRegions` with `covers`, `complete`, `add` and `missingFor`. Pure — no Monaco, no IPC — so the arithmetic that decides what to fetch is testable without a window (data-model.md, *LoadedRegions*)
- [X] T005 [P] `tests/unit/editor-ranges.test.ts`: ranges merge on insert, stay ascending and non-overlapping, `complete` is true only at full coverage, and a short response narrows rather than corrupts the record. Written before T004's body
- [X] T006 [P] `client/ui/lib/editor/palette.ts`: `monacoTheme(el)` reading the five `--vk-code-*` tokens plus background, foreground, selection and cursor from the mounted element, returning Monaco's theme object. The only place any editor colour is decided
- [X] T007 [P] `tests/unit/editor-palette.test.ts`: every colour in the returned theme traces to a token, no literal appears, and a role the prototype does not define falls back to the foreground rather than to a guess (A-EDITPALETTE)
- [X] T008 `client/ui/lib/editor/buffers.svelte.ts`: `Buffer` and `BufferSet` per data-model.md. **Module-level, not component state** — the panel is unmounted on every tab switch, and a model held in the component loses the buffer, its base and its dirty flag with it. This is the defect the terminal had when `detach()` disposed its instance; it is designed out here rather than found later
- [X] T009 [P] `tests/unit/editor-buffers.test.ts`: one buffer per path however many times it is opened (FR-023); `dirty` set by an edit and cleared only by `adopt` or `reload`; a buffer with no base refuses to save; `editable` false while regions are missing
- [X] T010 `client/ui/lib/editor/sink.ts`: the `EditorSink` port — `read`, `write`, `hash` — with an `overIpc` implementation and a `setEditorSink` swap for tests, following `terminal/sink.ts`, which is the established shape for this seam
- [X] T011 `client/ui/lib/editor/EditorPanel.svelte`: mounts Monaco into the document area, themed from T006, with language web workers **disabled** (FR-004). Renders the buffer for the focused tab and nothing when no document is open
- [X] T012 Wire `EditorPanel` into `client/ui/lib/shell/Window.svelte`'s document area, replacing the `No document open` placeholder when a tab is focused

---

## Phase 3: User Story 1 — Open a file and type into it (P1)

**Goal**: a file opens from the cache and renders every keystroke with nothing in the path.

**Independent test**: open a cached file, type a burst, confirm all characters render and zero
requests are issued.

- [ ] T013 [US1] `tests/e2e/editor-local-echo.spec.ts`: open a cached file, type 100 characters, assert all 100 are in the buffer and **zero** requests were issued as a result (SC-001, SC-002). Written first — this is the feature's central claim and §1.4's absolute rule
- [X] T014 [US1] In `client/ui/lib/editor/sink.ts`, extend the automation recorder to count requests to log every call, so SC-001 counts what actually left rather than what a component believed it sent. Following `terminal/sink.ts`'s `recordForAutomation`
- [X] T015 [US1] `client/core/src/adapters/inbound/tauri_commands.rs`: `file_read` returning content and hash for a path, reading through the existing `CachedWorkspace` so a cached file costs no request
- [X] T016 [US1] `client/ui/lib/editor/buffers.svelte.ts`: `open(path)` populating a buffer from the sink, setting `base` from the returned hash, and returning the existing buffer when one is already open for that path
- [X] T017 [US1] `EditorPanel.svelte`: bind Monaco's model to the buffer so an edit updates `text` and sets `dirty`, and confirm by construction that the edit path calls no sink method
- [X] T018 [US1] In `client/ui/lib/editor/buffers.svelte.ts`, decline content that is not valid UTF-8 (FR-006), with a message naming F017 as what will render it. The check belongs where the bytes arrive, not in the component
- [X] T019 [US1] Decline a file above the maximum opened as text (plan.md, *Fixed Quantities*), naming the limit. A window that stops responding is worse than a refusal
- [X] T020 [US1] `tests/e2e/editor-open.spec.ts`: US1's remaining acceptance scenarios — an uncached file is fetched once; a binary file is declined; the buffer survives the connection dropping and typing continues (FR-003); **syntax highlighting is still applied while disconnected** (FR-005), because it is local by design and a requirement satisfied only by accident is one a later change removes unnoticed; and **every character typed while disconnected is still present when the connection returns** (SC-009)

---

## Phase 4: User Story 2 — Save, and be told when the file moved underneath (P1)

**Goal**: a save writes through, and a stale save is refused without overwriting.

**Independent test**: save with a matching base and observe the new hash; change the file on the
host, save again, observe the refusal and the host's content unchanged.

### The protocol and the engine

- [X] T021 [P] [US2] `protocol/src/wire.rs`: `WriteFileParams` (`workspace_id`, `relative_path`, `content`, `base_sha256`) and `WriteFileResult` (`sha256`), matching contracts/write-file.md and §4.8
- [X] T022 [P] [US2] `protocol/src/lib.rs`: `codes::WRITE_CONFLICT = -32004`, which §4.4 specifies and the crate does not yet define
- [X] T023 [P] [US2] `protocol/tests/write_wire.rs`: the params and result round-trip, and the wire spelling is snake_case per §4.8 and A-WIRECASE. A hand-written frame, so the test would catch a rename the structs made silently
- [X] T024 [US2] `engine/tests/write_file.rs`: the integration suite, written before the implementation. A matching base writes and returns the hash of what landed; a mismatched base returns `-32004` **and the file on disk is byte-for-byte what it was**; a path escaping the root returns `-32003`; content above the bound is refused. Assert on the filesystem, not only on the reply — a test that reads the reply passes for an engine that refuses and writes anyway
- [X] T025 [US2] `engine/src/application/ports/file_system.rs`: add `write_atomic` and `hash_file` to the port. Capabilities, not technologies (Principle VIII)
- [X] T026 [US2] `engine/src/adapters/outbound/std_fs.rs`: implement `write_atomic` as write-to-temp-then-rename, with the temp file in the **destination's own directory** so the rename cannot cross a filesystem and degrade into a copy, and the target's mode preserved so saving does not strip an executable bit (contracts/write-file.md, guarantees 2 and 6)
- [X] T027 [US2] `engine/src/application/use_cases/workspace.rs`: `write_file` — resolve and contain the path through the existing `resolve_request`, bound the content, hash the current file, compare with `base_sha256`, refuse `Conflict` on mismatch **before opening anything for writing**, otherwise write and return the hash of what was written
- [X] T028 [US2] `engine/src/adapters/inbound/rpc.rs`: the `workspace/writeFile` dispatch arm, replacing the `METHOD_NOT_FOUND` the test at line 902 currently asserts — and update that test, which exists precisely to say the method is not implemented yet
- [X] T029 [US2] In `engine/src/application/use_cases/workspace.rs`, log each write and each conflict with the path and both hashes. A conflict a developer disputes afterwards is a support call with nothing to look at

### The client

- [X] T030 [P] [US2] `client/core/tests/write_file.rs`: `RemoteWorkspaceProvider::write_file` maps a success to the new hash, `-32004` to a conflict, and every other code to its own outcome. Against a scripted transport, so each branch is exercised without an engine
- [X] T031 [US2] `client/core/src/adapters/outbound/remote_workspace.rs`: implement `write_file`, removing the `Owner::F006Editor` refusal F003 left
- [X] T032 [US2] `client/core/src/application/use_cases/edit_file.rs`: map a provider result to `WriteOutcome`'s four variants. `Conflict` and `Unreachable` are never collapsed — one means a colleague edited the file, the other means the link dropped, and the developer's next action differs completely (FR-012)
- [X] T033 [US2] `client/core/src/adapters/inbound/tauri_commands.rs`: `file_write`, validating its arguments in the core because the webview is not a trusted caller (Principle VI)
- [X] T034 [US2] In `client/core/src/application/use_cases/edit_file.rs`, update the workspace cache after a successful write so a reopen does not show stale content (FR-010)

### The surface

- [X] T035 [P] [US2] `client/ui/lib/editor/ending.ts`: `describeOutcome` turning a `WriteOutcome` into what the developer is told. Pure, so the wording and the distinctions are testable without a window
- [X] T036 [P] [US2] `tests/unit/editor-ending.test.ts`: a conflict reads as somebody else's edit, an unreachable engine reads as the link, and neither is ever rendered as the other
- [X] T037 [US2] `EditorPanel.svelte`: an explicit save action (FR-007a) sending the buffer's text and base through the sink, and adopting the returned hash on success
- [X] T038 [US2] In `client/ui/lib/editor/buffers.svelte.ts`, refuse a second save while one is in flight for the same buffer (FR-014), so two writes cannot race into a wrong base
- [X] T039 [US2] In `client/ui/lib/editor/buffers.svelte.ts`, keep the buffer `dirty` until a write's response arrives, and leave it dirty **with its text untouched** on any outcome that is not `Written` (FR-013, FR-011). **The failing case is an optimistic clear**: marking the buffer saved when the request goes out satisfies every other task in this phase and tells the developer their work is on the host when it is in flight, or lost. Asserted in `tests/unit/editor-buffers.test.ts` against an outcome that never resolves and against `Unreachable`
- [X] T040 [US2] In `client/ui/lib/editor/EditorPanel.svelte`, present a refused save with exactly one way out: discard the local changes and reload the host's content (FR-012a). Explicitly **not** an overwrite — re-reading the hash and writing over it destroys a colleague's work silently, which §11 names as the failure this product cannot afford (FR-012b)
- [X] T041 [US2] In `client/core/src/domain/session.rs` and `client/ui/lib/editor/buffers.svelte.ts`, autosave: a preference in the session store, **off** when unset, a debounce per plan.md's fixed quantities, and no write for a buffer with no changes (FR-007b, FR-007c). Raise the store's schema version with `serde(default)`, the migration A-STATE2 established
- [X] T042 [US2] In `client/ui/lib/editor/EditorPanel.svelte`, one control to turn autosave on. Without it the mechanism is unreachable and therefore dead code, which this project does not ship (research.md, last entry)
- [ ] T043 [US2] `tests/e2e/editor-save.spec.ts`: US2's acceptance scenarios end to end against a real engine — a clean save, a refused save with the host's bytes unchanged, a save with no connection, a reopen showing what was saved, **discarding after a conflict leaving the buffer holding exactly the host's bytes** (SC-015) — the one escape this feature offers, and until now the only one with no test — and autosave issuing zero writes when off (SC-003, SC-004, SC-014, SC-015)

### Not hearing our own write

- [X] T044 [US2] `client/ui/lib/editor/buffers.svelte.ts`: on a file event for an open file, ask for the file's current hash and compare with the buffer's base before treating it as a change (A-WRITEECHO, FR-024, FR-024a)
- [X] T045 [US2] In `client/ui/lib/editor/buffers.svelte.ts`, a genuinely diverged file with a **clean** buffer may refresh; with a **dirty** buffer it must not be replaced and the unsaved changes must survive (FR-024b)
- [X] T046 [US2] In `client/ui/lib/editor/buffers.svelte.ts`, report a deletion of an open file without discarding the buffer (FR-025)
- [ ] T047 [US2] `tests/e2e/editor-echo.spec.ts`: saving produces **zero** "changed on the host" notices (SC-012), and a change made by something other than the client **is** reported (SC-013). Both, because each is the other's failure mode — a fix for one that breaks the other looks correct from whichever side you are standing on

---

## Phase 5: User Story 3 — Open a very large file without waiting (P2)

**Goal**: the first screen renders from a partial read; the rest follows the viewport.

**Independent test**: open a file above the threshold, confirm the first window renders before
the whole file has transferred, and that scrolling fetches more.

- [X] T048 [P] [US3] `client/core/src/adapters/inbound/tauri_commands.rs`: `file_read_range`, passing a byte range through to the provider's existing ranged `read_file`
- [X] T049 [US3] `client/ui/lib/editor/buffers.svelte.ts`: open a file above the threshold by reading only the range covering the first viewport, recording it in `LoadedRegions`, and leaving the buffer `editable === false`
- [X] T050 [US3] In `client/ui/lib/editor/EditorPanel.svelte`, fetch the next range when the viewport moves beyond what is loaded (FR-017), using T004's `missingFor` to ask for exactly what is absent
- [X] T051 [US3] In `client/ui/lib/editor/buffers.svelte.ts`, read a file at or below the threshold whole, in one request (FR-018) — a range request for a small file costs the same round trip and delivers less
- [X] T052 [US3] In `client/ui/lib/editor/EditorPanel.svelte`, refuse edits to a partially loaded buffer, visibly, and load the remainder when the developer asks to edit. A whole-file write of a partial buffer would replace the unloaded regions with nothing (research.md, *Ranges*)
- [ ] T053 [US3] `client/core/tests/editor_first_paint.rs`: p99 over at least 100 samples from open to the first window being available, **printed** against 250 ms, with the chunk count and the largest response recorded (SC-005, SC-006, SC-007, A-NFR)
- [ ] T054 [US3] `tests/e2e/editor-large-file.spec.ts`: US3's acceptance scenarios — the first window renders, scrolling fetches more, no response exceeds the frame limit, and a small file is read whole

---

## Phase 6: User Story 4 — Come back to what was open (P2)

**Goal**: the tabs F000 restores have content behind them.

**Independent test**: open three files, focus the second, relaunch, confirm three tabs in order
with the second focused and showing its content.

- [X] T055 [US4] In `client/ui/lib/editor/EditorPanel.svelte`, restore a focused tab's content on relaunch, from the cache where valid and from the engine otherwise (FR-021). The tab list itself is already restored by F000; this gives it something behind it
- [X] T056 [US4] In `client/ui/lib/editor/buffers.svelte.ts`, report a restored tab whose file no longer exists rather than presenting an empty buffer (FR-022)
- [ ] T057 [US4] `tests/e2e/editor-session.spec.ts`: US4's acceptance scenarios — tab count, order and focus identical after a relaunch (SC-008), content shown when focused, and a vanished file reported

---

## Phase 7: Polish and cross-cutting

- [ ] T058 [P] `tests/e2e/editor-a11y.spec.ts`: the conflict notice and the autosave control are reachable from the keyboard and legible in greyscale, following `rail-greyscale.spec.ts` — comparing what survives desaturation rather than comparing hues, since a test that compared colours would pass for a design that relied on them
- [ ] T059 [P] Update `docs/app-shell.md` with the editor surface: where the buffer model lives and why it outlives the component, what the palette translates, and the partial-buffer interlock
- [ ] T060 [P] Update `docs/engine.md` with the write path: containment reused rather than re-implemented, compare-before-open, and rename-over-write with the temp file in the destination's directory
- [ ] T061 Per `specs/008-editor-integration/quickstart.md` §5, run the seven mutation checks in [quickstart.md](./quickstart.md) §5 and record each outcome. Each must fail **with the assertion expected** rather than with a compile error. Mutations 4 and 5 matter most: they are each other's failure mode, and a fix for one that breaks the other looks correct from either side
- [ ] T062 Per `specs/008-editor-integration/quickstart.md` §4, audit the seven negative checks in [quickstart.md](./quickstart.md) §4 and confirm, for each, that the fixture condition which lets it fail is actually present — a real symlink rather than a string with `..` in it, a watcher actually running, a file genuinely changed on disk
- [ ] T063 In `specs/008-editor-integration/quickstart.md`, record the six measurements from [quickstart.md](./quickstart.md) §3 in its *Validation record*, each number beside its bound. A gate that says only PASS tells nobody how much headroom is left
- [ ] T064 Verify `make gate` is green, then update `specs/features-map.md`, then mark F006 complete in `specs/features-map.md` and run `feature_map.py verify`

---

## Dependencies

```
Phase 1 Setup
  └─▶ Phase 2 Foundational (the surface exists)
        ├─▶ Phase 3  US1  open and type        (P1)
        ├─▶ Phase 4  US2  save and conflict    (P1, needs US1's buffer)
        ├─▶ Phase 5  US3  large files          (P2, needs US1's open path)
        └─▶ Phase 6  US4  session restore      (P2, needs US1's open path)
                          └─▶ Phase 7 Polish
```

US2, US3 and US4 all depend on US1 because each needs a buffer that opens. They do not depend on
each other and can proceed in parallel once US1 lands.

---

## Parallel opportunities

- **Phase 2**: T004/T005, T006/T007 and T009 touch different files with no shared state.
- **Phase 4**: the protocol tasks (T021–T023), the client mapping test (T030) and the pure
  surface modules (T035, T036) are independent of the engine work (T024–T029).
- **Phase 7**: T058, T059 and T060 are three different files.

---

## Implementation strategy

**MVP is US1 alone**: a file that opens and types with nothing in the path. That is the product's
central claim and §1.4's one absolute rule, and it is worth having working before anything writes
to a filesystem.

US2 is the riskiest phase and carries most of the tests, because it is the first thing in this
system that can destroy a developer's work. Its engine tests are written before its engine code,
which Principle VII prefers wherever a wrong answer is expensive.

US3 and US4 are each a small slice on top of US1's open path and could be dropped from a first
release without making the editor incoherent — a large file would simply be slow, and tabs would
come back empty. Neither is true of US1 or US2.
