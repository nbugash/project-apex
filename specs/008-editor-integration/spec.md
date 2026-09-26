# Feature Specification: Editor Integration

**Feature Branch**: `feature/F006-editor-integration`

**Created**: 2026-09-26

**Status**: Draft

**Input**: F006 `editor-integration` from `specs/features-map.md` — Monaco text model bound to
cached content with local echo; write path with `baseSha256` conflict rejection; large-file
chunked loading on scroll; tab and layout state persistence.

---

## On the source of values

Every number and every method name in this specification is read from
`project-apex-predator.md` or from the code that already exists, and says which. Where this
document fixes a value the system specification does not state, it says so in *Assumptions* and
gives the reasoning, so a reviewer can tell a derived constraint from an invented one.

---

## What this feature is not

- **Not offline editing.** A-OFFLINE leaves the editor writable when the connection drops and
  merges on reconnect. That is F012 `offline-editing` and F019 `offline-merge`. This feature
  writes through to the engine and reports failure when it cannot; it persists no unsaved buffer
  across a restart and performs no merge.
- **Not conflict resolution.** §11's conflict interface and three-way merge belong to F012.
  What this feature owns is the **rejection**: the engine refuses a stale write with `-32004`
  and the client tells the developer, without overwriting anything.
- **Not language intelligence.** Completion, hover, definition, references and diagnostics
  translate into `lsp/request` and are F007 `lsp-multiplexing`. Monaco's own web workers are
  disabled here so that they cannot silently answer with a local model of a remote workspace,
  but nothing replaces them yet.
- **Not a binary viewer.** §9.2 renders images, PDFs and generated media; F017
  `previews-artifacts` owns that. This feature recognises content it cannot present as text and
  declines it, rather than showing mojibake.
- **Not file management.** `create_file`, `create_directory`, `rename` and `delete` are declared
  in §6.1 and refused by the provider with their own owners. Only `write_file` moves here.

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Open a file and type into it (Priority: P1)

A developer opens a file from the project tree and edits it. Characters appear as fast as they
would in a local editor, whatever the link is doing.

**Why this priority**: it is the product's central claim. §1.4 budgets keystroke-to-glyph at
**0 ms network**, and §1.5's first rule says the engine is never in the path of rendering. An
editor that fails this is the thing this architecture exists to avoid.

**Independent Test**: open a cached file, type a burst of characters, and confirm every one is
rendered and that **no request left the client** while typing.

**Acceptance Scenarios**:

1. **Given** a file whose content is in the cache, **When** the developer opens it, **Then** the
   editor shows that content without asking the engine for it.
2. **Given** a file not in the cache, **When** the developer opens it, **Then** the client
   fetches it once and shows it.
3. **Given** an open file, **When** the developer types 100 characters, **Then** all 100 are
   rendered and **zero** requests are issued as a result of the typing.
4. **Given** an open file, **When** the connection is lost, **Then** typing continues to render
   and the buffer is not cleared.

---

### User Story 2 - Save, and be told when the file moved underneath (Priority: P1)

A developer saves. If the file on the host changed since they opened it, the save is refused and
they are told — rather than silently overwriting someone else's work.

**Why this priority**: §4.8 carries `baseSha256` for exactly this, and the remote side is where
CI and colleagues write. A last-writer-wins save is the one failure that destroys work without
telling anyone.

**Independent Test**: save a file whose base hash still matches and observe the new hash; change
the file on the host, save again, and observe the refusal with the remote content unchanged.

**Acceptance Scenarios**:

1. **Given** an unmodified base, **When** the developer saves, **Then** the write succeeds and
   the returned hash becomes the buffer's new base.
2. **Given** the file changed on the host since it was opened, **When** the developer saves,
   **Then** the engine refuses with `-32004`, the host's content is unchanged, and the
   developer's buffer is intact.
3. **Given** a refused save, **When** the developer looks at the editor, **Then** the reason is
   distinguishable from a transport failure — one is somebody else's edit, the other is the
   link.
4. **Given** no connection, **When** the developer saves, **Then** the save fails and says so,
   and the buffer is not marked saved.
5. **Given** a successful save, **When** the file is closed and reopened, **Then** the content
   shown is what was saved.

---

### User Story 3 - Open a very large file without waiting for it (Priority: P2)

A developer opens a multi-megabyte log or generated source file and starts reading immediately.

**Why this priority**: §4.6 makes one pipe one queue, so fetching a whole large file ahead of
first paint blocks everything behind it. This is what the ranged form of `workspace/readFile`
exists for, and it is already implemented on both sides.

**Independent Test**: open a file larger than the chunk threshold and confirm the first screen
renders from a partial read, with later ranges fetched as the developer scrolls.

**Acceptance Scenarios**:

1. **Given** a file above the threshold, **When** it is opened, **Then** the first visible
   window renders without the whole file having been transferred.
2. **Given** such a file, **When** the developer scrolls beyond what is loaded, **Then** the
   next range is fetched and rendered.
3. **Given** such a file, **When** ranges are fetched, **Then** no single response exceeds the
   frame limit §4.1 fixes.
4. **Given** a file below the threshold, **When** it is opened, **Then** it is read once and
   whole, because a range request for a small file costs a round trip and saves nothing.

---

### User Story 4 - Come back to what was open (Priority: P2)

A developer quits with several files open and relaunches to find the same tabs, in the same
order, with the same one focused.

**Why this priority**: A-STATE already persists open document references, order and focus, and
F000 restores the tab strip. What is missing is that the tabs have no content behind them —
the editor was never built. This closes that.

**Independent Test**: open three files, focus the second, relaunch, and confirm three tabs in
the same order with the second focused and showing its content.

**Acceptance Scenarios**:

1. **Given** several open files, **When** the application is relaunched, **Then** the same tabs
   appear in the same order with the same one focused.
2. **Given** a restored tab, **When** it is focused, **Then** its content is shown — from cache
   where valid, otherwise fetched.
3. **Given** a restored tab whose file no longer exists on the host, **When** it is focused,
   **Then** the developer is told it is gone rather than shown an empty editor.

---

### Edge Cases

- **The file changes on the host while it is open.** F004 emits `workspace/onFileEvent`. The
  editor must notice, because a save that follows would be refused and the developer should know
  before they are told by a rejection. An unmodified buffer may be refreshed; a modified one
  must not be overwritten without asking.
- **The file is deleted on the host while it is open.** The buffer is not discarded; the
  developer is told.
- **Content is not valid UTF-8.** §6.1 returns bytes precisely because build artifacts and
  images are legal content. The editor declines to present such a file as text.
- **A file is opened twice.** One buffer, not two — two buffers of one file means two bases and
  a save that silently reverts the other.
- **A save is issued while an earlier save for the same file is still in flight.** The second
  must not race the first into a wrong base.
- **A range request returns fewer bytes than asked for**, because the file shrank between
  ranges.
- **The connection drops mid-save.** The write is not complete until its response arrives
  (§6.1), so it must not be reported as saved.

---

## Requirements *(mandatory)*

### Functional Requirements

**Opening and rendering**

- **FR-001**: The editor MUST populate its buffer from the workspace cache when the cache holds
  valid content for the file, and from the engine otherwise.
- **FR-002**: Rendering a keystroke MUST NOT depend on any engine request. Typing MUST issue no
  request of any kind.
- **FR-003**: The editor MUST continue to accept and render input while the connection is down.
- **FR-004**: Monaco's local language web workers MUST be disabled, so that no completion,
  hover, or diagnostic can be answered from a local model of a remote workspace.
- **FR-005**: Syntax highlighting MUST survive a dropped connection, because it is local.
- **FR-006**: Content that is not valid UTF-8 MUST be declined rather than presented as text,
  naming the feature that will present it.

**Writing**

- **FR-007**: A save MUST carry the content and the `baseSha256` the client held when the buffer
  was last known to match the host.
- **FR-008**: The engine MUST compare `baseSha256` against the file's current content and MUST
  refuse a mismatch with `-32004` **without writing**.
- **FR-009**: A successful write MUST return the new `sha256`, and the client MUST adopt it as
  the buffer's new base.
- **FR-010**: A successful write MUST leave the cache holding what was written, so that a reopen
  does not show stale content.
- **FR-011**: A refused write MUST leave the developer's buffer intact and unmarked as saved.
- **FR-012**: A write conflict MUST be distinguishable, to the developer, from a transport
  failure and from a permission failure.
- **FR-013**: A write MUST NOT be reported as saved until its response has arrived.
- **FR-014**: Two saves of one file MUST NOT be in flight at once.
- **FR-015**: A write that fails MUST leave the file's previous content byte-for-byte intact.
  A partly-written file is never an acceptable outcome, and "intact" is what a test can read
  back — where "atomic" would only describe how it was achieved.

**Large files**

- **FR-016**: A file whose size exceeds the chunk threshold MUST render its first visible window
  from a ranged read rather than from a whole-file read.
- **FR-017**: Scrolling beyond the loaded region MUST fetch the next range.
- **FR-018**: A file at or below the threshold MUST be read whole, in one request.
- **FR-019**: No single read response may exceed the frame limit of §4.1.

**Session**

- **FR-020**: Open documents, their order and the focused one MUST survive a restart, using the
  store A-STATE established.
- **FR-021**: A restored tab MUST show its content when focused.
- **FR-022**: A restored tab whose file is gone MUST say so rather than presenting an empty
  buffer.
- **FR-023**: One file MUST have at most one buffer, however many times it is opened.

**Reacting to the host**

- **FR-024**: When a file event reports that an open file changed on the host, a **modified**
  buffer MUST NOT be replaced and its unsaved changes MUST survive the event. An **unmodified**
  buffer MAY be refreshed; that half is a permission rather than an obligation, so the checkable
  requirement is stated on the side that can be violated.
- **FR-025**: When a file event reports that an open file was deleted, the developer MUST be
  told and the buffer MUST NOT be discarded.

### Key Entities

- **Buffer** — one open file's text, the base hash it was read at, and whether it has unsaved
  changes. At most one per file.
- **Base revision** — the `sha256` the content had when it was read or last written. What a save
  is checked against.
- **Loaded region** — for a large file, which byte ranges the buffer currently holds.

---

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Typing 100 characters into an open buffer issues **zero** requests, and all 100
  are rendered. §1.4's keystroke budget is "0 ms network", so the observable is the count, not a
  duration.
- **SC-002**: Opening a cached file issues **zero** read requests, in 100% of exercised cases.
- **SC-003**: A stale save is refused in 100% of exercised cases, and the host's content is
  byte-for-byte unchanged after the refusal.
- **SC-004**: A save whose base still matches succeeds in 100% of exercised cases, and reopening
  the file returns exactly the bytes written.
- **SC-005**: The first visible window of a file of **at least eight chunk thresholds** renders
  at p99 under **250 ms**, measured at the interface boundary over at least 100 samples, with the
  measured value printed (§1.4, A-NFR). Stated in thresholds rather than in megabytes so the
  criterion still means "many chunks" if the threshold moves.
- **SC-006**: Opening such a file transfers no more than one chunk before first paint.
- **SC-007**: No read response exceeds 1 MiB (§4.1).
- **SC-008**: After a relaunch, the number of tabs, their order and the focused one are
  identical to what they were, in 100% of exercised cases.
- **SC-009**: Editing remains possible with the connection down: 100 characters typed while
  disconnected are all rendered, and none is lost when the connection returns.
- **SC-010**: A file opened twice yields one buffer, in 100% of exercised cases.
- **SC-011**: Content that is not valid UTF-8 is never rendered as text; the refusal names what
  will render it.

---

## Assumptions

- **The chunk threshold is 1 MiB.** The system specification fixes the frame limit at 1 MiB
  (§4.1) and requires ranged reads for "large files" (§4.6) without defining large. One frame is
  the natural boundary: a file that fits in a frame costs one round trip whole, and a range
  request for it would cost the same round trip and deliver less. Recorded here because it is
  this feature fixing a number the system left open; `plan.md` will state it as a fixed quantity.
- **Editor state beyond tabs, order and focus does not persist.** A-STATE names window geometry,
  region layout, open document references and focus. Cursor position and scroll offset are not
  named, and no requirement asks for them, so they are not built. Adding them is cheap later and
  inventing them now would be building what nothing asked for.
- **Unsaved edits do not survive a restart.** A-OFFLINE makes locally persisted edits F012's,
  against the `baseSha256` held when the connection dropped. Persisting them here would build
  half of F012 in the wrong place.
- **A refused write leaves recovery to the developer.** Showing the difference and offering to
  merge is F012's conflict interface. Here the developer is told, and their buffer is kept.
- **The engine's write is whole-file.** §4.8's `writeFile` carries `content`, not a patch.
  Incremental writes are not in the protocol and are not assumed.

---

## Dependencies

- **F000 `app-shell`** — the tab strip, the layout, and the durable session store A-STATE
  defines. Complete.
- **F003 `workspace-cache`** — `read_file` with ranges, the SQLite projection, and the
  `WorkspaceProvider` port whose `write_file` is declared and refused with `Owner::F006Editor`.
  Complete.
- **F004 `file-watch-sync`** — `workspace/onFileEvent`, which FR-024 and FR-025 consume.
  Complete, and reachable since F010 gave engine-initiated frames a route (A-NOTIFYROUTE).
- **F010 `execution-terminals`** — not a dependency of record, but it built the client's
  `RequestSender`, the engine-notification route and the local engine mode this feature's tests
  will use. Before it, no feature could reach an engine at all.
