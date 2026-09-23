---

description: "Task list for F003 workspace-cache"
---

# Tasks: Workspace Cache

**Input**: Design documents from `/specs/005-workspace-cache/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/](./contracts/), [architecture.md](./architecture.md),
[design.md](./design.md)

**Tests**: **Included, and not optional.** Constitution Principle VII requires automated tests at
every level where a feature has surface, and this one has all three. Where the constitution names
fail-first — "protocol framing, path containment, cache validity" — the test task is ordered before
its implementation task and says so.

**Organization**: Grouped by user story. Each story is independently testable once Phase 2 is done.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel — different files, no dependency on an incomplete task
- **[Story]**: US1–US5, mapping to the user stories in spec.md
- Exact file paths in every description

## Path Conventions

Cargo workspace with three crates plus a webview layer, per plan.md's Structure Decision:
`protocol/`, `engine/`, `client/core/` (crate `apex-shell`), `client/ui/`, `tests/e2e/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Close the four gaps in the system specification, promote the three decisions that
bind later features, and add the dependencies. Principle II requires the first before dependent
work starts; Principle III requires the second before the code implementing it.

- [X] T001 Add `workspace/register` to the Workspace table in §4.8 of `project-apex-predator.md`, with params `workspaceId`, `path` and result `{name, canonicalPath}`, per [contracts/workspace-methods.md](./contracts/workspace-methods.md). State that the root is canonicalised once at registration, that re-registering the same id against the same path is idempotent and against a different path is an error, and that the registry is in-memory so a client re-registers after an engine restart. Note explicitly that adding a method does **not** increment `protocolVersion` under §4.8's own rule, so an older engine answers `-32601` and the client redeploys. **This is the gap that makes every other workspace method unusable** — §15.4 step 3 and §4.4's `-32001` both presume this method and nothing defines it
- [X] T002 Add `cursor?` and `limit?` params and a `nextCursor?` result field to `workspace/readDirectory` in §4.8 of `project-apex-predator.md`. **State the entry ordering — `(type DESC, name ASC)`, byte-wise on UTF-8 — as contractual**, because the cursor depends on it and a later change to the sort order would therefore be breaking and *would* increment `protocolVersion`. Record the 1000-entry default and maximum (FR-024)
- [X] T003 Add the three external-content synchronisation triggers to §5.2 of `project-apex-predator.md`, exactly as written in [data-model.md](./data-model.md). Add a fourth bullet to §5.2's "Three corrections are load-bearing" list — renaming it — explaining that `files_fts` is external-content and SQLite does not maintain it, so without triggers the index is created empty, stays empty, and every offline path search returns nothing with no error
- [X] T004 [P] Add decision `A-BULKSIZE` to Appendix A of `project-apex-predator.md` dated 2026-09-23: the bulk threshold is 512 KiB of raw payload, with the base64 4:3 arithmetic that rules out 768 KiB, the reconciliation of FR-023 with FR-025, and the reversal condition. Reference A-BULK rather than restating it
- [X] T005 [P] Add decision `A-CACHECAP` to Appendix A of `project-apex-predator.md` dated 2026-09-23: content above 8 MiB is read but never cached. **State the consequence plainly — a file above the cap is never available offline** — and the rejected alternatives (no cap, a total-size LRU budget which contradicts §5.5 and A-WORKSPACE, a cap derived from free disk)
- [X] T006 [P] Add decision `A-DEADLINE` to Appendix A of `project-apex-predator.md` dated 2026-09-23: a request on the interaction path states its own timeout derived from the §1.4 budget rather than taking the transport default. The confirmation limit of 2 seconds is its first instance, with the reasoning that a limit near 250 ms would expire in normal operation and train developers to ignore the marker
- [X] T096 Add error code `-32009` — "Workspace root no longer exists" — to the §4.4 table in `project-apex-predator.md`, and state that it is distinct from `-32001` because the two demand **opposite client responses**: `-32001` means re-register, `-32009` means tell the developer and stop presenting the projection. Overloading `-32001` would make a deleted workspace trigger a re-registration that then fails on `workspace/register`'s not-a-directory refusal, surfacing a registration error for a deletion (FR-038). Adding an error code does not increment `protocolVersion` — an older engine never sends it
- [X] T007 Add the new dependencies: `rusqlite` with features `["bundled"]` — **not** `fts5`, which is not a rusqlite feature; FTS5 is compiled into the SQLite that `bundled` builds, and T027 relies on that, `zstd`, `sha2` and `async-trait` to `client/core/Cargo.toml`; `sha2` alone to `engine/Cargo.toml`. **Do not add tokio to the engine** — research.md, "The engine stays synchronous". Verify `cargo build --workspace` succeeds and record the engine binary's size before and after, since it is transferred on every first connect
- [X] T008 [P] Confirm the screenshot segment resolves to `F003` from the branch `feature/F003-workspace-cache` by running `npm run e2e` against F002's existing specs and checking captures land in `reports/screenshots/${OS}/F003/`. F002 made `featureSegment()` branch-derived and `capturedFiles()` recursive; this verifies both still hold rather than assuming they do

**Checkpoint**: The system specification is complete enough to build against, the three promoted
decisions are recorded, and the workspace compiles with its new dependencies.

---

## Phase 2: Foundational (Blocking Prerequisites)

**⚠️ CRITICAL**: No user story work begins until this phase is complete. Registration is here, not
in US3, because the engine cannot answer any workspace method for a `workspaceId` it has never
been told about — so US1 would be untestable without it. US3 owns the *semantics* of identity;
this phase owns the mechanism.

### Shared wire types

- [X] T009 Define the workspace wire types in `protocol/src/wire.rs` per [data-model.md](./data-model.md) and [contracts/workspace-methods.md](./contracts/workspace-methods.md): `RegisterParams`/`RegisterResult`, `ReadDirectoryParams` (with `cursor`, `limit`)/`ReadDirectoryResult` (with `nextCursor`), `StatParams`/`StatResult`, `ReadFileParams`/`ReadFileResult`, and `FsEntryWire`. Serde-only, no logic — this crate links into both binaries
- [X] T010 [P] Add the §4.4 application error codes as named constants in `protocol/src/wire.rs`: `WORKSPACE_NOT_REGISTERED` (-32001), `PATH_REFUSED` (-32002), `NOT_FOUND` (-32003), `PAYLOAD_TOO_LARGE` (-32007), `WORKSPACE_GONE` (-32009). Both ends must agree on these and neither may write the integer inline

### Client domain

- [X] T011 [P] Implement `WorkspaceId`, `Workspace`, `Location`, `FileId` and `Sha256` in `client/core/src/domain/workspace.rs` per [data-model.md](./data-model.md). `Sha256` renders lowercase hex on the wire and is constructed from bytes, never from a hand-written string
- [X] T012 Implement `RelPath` in `client/core/src/domain/workspace.rs` with lexical validation on construction: no `..` component, not absolute, normalised to a leading `/` with no trailing slash except the root. Include the unit tests for the rejection cases — this is the client half of FR-008, and its own docs must say it is not what makes the system safe
- [X] T013 Implement `FsEntry`, `FsMeta`, `ByteRange`, `FileChunk`, `Page` and `DirPage` in `client/core/src/domain/workspace.rs`. `FileChunk.sha256` is documented as the **whole file's** hash, never the range's (FR-021)
- [X] T014 [P] Implement `CacheEntry`, `Validity`, `Presentation`, `MaintenancePhase` and `RetentionWindow` in `client/core/src/domain/cache.rs` per [data-model.md](./data-model.md). `Validity` has exactly one constructor, taking two hashes — there must be no path by which git status can reach it (FR-020, §5.3)

### Client ports

- [X] T015 [P] Define the `WorkspaceProvider` port in `client/core/src/application/ports/workspace_provider.rs` with `#[async_trait]` and the full §6.1 method set, plus `ProviderError` with variants `NotFound`, `Refused`, `UnknownWorkspace`, `WorkspaceGone`, `Offline`, `TooLarge`, `Transport` and `Unsupported { owner }`. **`UnknownWorkspace` and `WorkspaceGone` are separate variants, not one with a flag** — they lead to opposite responses (re-register versus tell the developer), and a flag is something a caller can forget to read. Signatures from [design.md](./design.md)
- [X] T016 [P] Define the `WorkspaceCache` port in `client/core/src/application/ports/workspace_cache.rs`. **`put_content` and `touch` return `StoreOutcome`, not `Result`** — a `Result` invites `?`, and `?` is how FR-034 gets violated by reflex rather than by decision. Document that on the trait
- [X] T017 [P] Define the `BulkTransfer` port in `client/core/src/application/ports/bulk_transfer.rs`, returning bytes with no integrity claim: the caller compares against the hash from `stat`
- [X] T018 [P] Define the `Clock` port in `client/core/src/application/ports/clock.rs` and implement `SystemClock` in `client/core/src/adapters/outbound/system_clock.rs`

### Engine — path safety first (fail-first, Principle VII)

- [X] T019 Write the path containment tests in `engine/tests/path_containment.rs` **before** the implementation, against a real temp-directory tree: `../../etc/passwd` refused; a symlink inside the workspace pointing outside it refused; an escape to a target that exists and one that does not producing the **same** error (FR-007); an unchecked path from a client still refused (FR-008). Confirm they fail
- [X] T020 Implement `ResolvedPath` and `PathRefusal` in `engine/src/domain/path.rs` with the two-stage check from research.md: lexical rejection before the filesystem is consulted, then `canonicalize` and a descendant assertion. **`ResolvedPath` has no public constructor other than `resolve`**, so a use case cannot name a path it has not checked. T019 must now pass
- [X] T021 [P] Define the engine's `FileSystem` port in `engine/src/application/ports/file_system.rs` — synchronous, with `canonicalize`, `read_dir`, `metadata`, `read_range` — and the `WorkspaceRoots` port in `engine/src/application/ports/roots.rs`
- [X] T022 [P] Implement `StdFileSystem` in `engine/src/adapters/outbound/std_fs.rs` over `std::fs`, and an in-memory `FakeFileSystem` in `engine/tests/common/mod.rs` so use cases are testable without a real tree

### Engine — hexagonal restructure

- [X] T023 Move the method dispatch out of `engine/src/main.rs` into `engine/src/adapters/inbound/rpc.rs`, carrying F002's `auth/handshake`, `session/restart` and `session/shutdown` across **unchanged**, including the `-32000` drain-before-exec behaviour. F002's engine tests must pass without edits — if any needs changing, the move was not behaviour-preserving
- [X] T024 Reduce `engine/src/main.rs` to a composition root and the stdio loop: construct `StdFileSystem`, the roots registry and the use cases, and hand them to the dispatch adapter. No business rule remains in `main.rs` (Principle VIII)
- [X] T025 Implement the in-memory `WorkspaceRoots` registry in `engine/src/application/use_cases/workspace.rs`, canonicalising each root once at registration, and the `Register` use case with the idempotence and refusal rules from [contracts/workspace-methods.md](./contracts/workspace-methods.md)
- [X] T026 Wire `workspace/register` into `engine/src/adapters/inbound/rpc.rs`, returning `-32001` for every workspace method called against an unregistered id, and `-32009` when the id is registered but its root no longer resolves

### The projection

- [X] T027 Implement the v1 schema in `client/core/src/adapters/outbound/sqlite/schema.rs`: the **whole** of §5.2 including `git_status`, plus the three FTS triggers from T003. Set `journal_mode = WAL`, `synchronous = NORMAL` and `foreign_keys = ON` **on every connection** — `foreign_keys` is per-connection in SQLite and defaults off, and a connection that forgets it silently breaks the cascade that gives FR-012
- [X] T028 Implement the migration ladder — the `migrate_to` operation of the `WorkspaceCache` port — in `client/core/src/adapters/outbound/sqlite/migrate.rs`: read `PRAGMA user_version`; each step in one transaction that also bumps the version; on failure or on a version newer than this build, delete the database file **and its `-wal` and `-shm` companions** and recreate. Deleting only the main file leaves a WAL SQLite will replay into the fresh one
- [X] T029 Implement `SqliteWorkspaceCache` in `client/core/src/adapters/outbound/sqlite/mod.rs`: one connection behind a mutex, every operation crossing `tokio::task::spawn_blocking`, per research.md. Implement only `register`, `forget` and `schema_version` here; the read and write operations land in their own stories

### Test doubles and the contract suites

- [X] T030 [P] Implement `FakeWorkspace` in `client/core/tests/common/fake_workspace.rs`: a path-to-bytes map, configurable latency, and switches for "never answers" and "content changed underneath". The never-answers switch is what makes FR-021c testable at all
- [X] T031 [P] Implement `InMemoryCache` in `client/core/tests/common/fake_cache.rs` reproducing C1–C6 from [contracts/cache.md](./contracts/cache.md), **including failure on demand**: a refusing `put_content` for FR-034 and a settable schema version for the migration tests. A fake that cannot fail tests only the happy path, and FR-034 does not live there
- [X] T032 [P] Implement `FakeClock` in `client/core/tests/common/fake_clock.rs` so retention is exercisable without waiting fourteen days
- [X] T033 Write the `WorkspaceProvider` contract suite in `client/core/tests/provider_contract.rs`, parameterised over the constructor and asserting P1–P6 from [contracts/provider.md](./contracts/provider.md), plus R1 for providers with an engine behind them. This is the mechanism by which "the UI never learns which is active" becomes a test rather than an intention. **Written before the implementations it will police.** At this phase only `FakeWorkspace` exists, so the suite is green against one implementation and gains the others as they land — `RemoteWorkspaceProvider` at T042, `CachedWorkspace` at T044. R1 is mutation-checked rather than assumed: inject a fan-out into the fake and confirm the assertion fails
- [X] T034 Write the `WorkspaceCache` contract suite in `client/core/tests/cache_contract.rs` asserting C1–C9, run against both `SqliteWorkspaceCache` (real file, temp dir) and `InMemoryCache`. Rust requires a trait impl to be complete, so the whole `WorkspaceCache` surface lands with T029 and this suite is green from the start rather than reddening across phases. **Do not weaken an assertion to make it pass** — this project has already shipped five tests that passed by proving nothing. Each guarantee's own story task still owns its behavioural tests

### Composition ordering

- [ ] T035 Extend `client/core/src/composition.rs` so the cache is constructed, maintenance is run, and **only then** are any providers built. Add an assertion or a type-level guard that makes the order impossible to get wrong (FR-018c, FR-026a). `MaintainCache` may be a version check only at this point; US4 fills it in
- [ ] T036 [P] Implement `RegisterWorkspace` in `client/core/src/application/use_cases/register_workspace.rs`: mint a `WorkspaceId` (A-WORKSPACE), attach to an existing projection if one exists, and call `workspace/register` on the engine

**Checkpoint**: Both binaries are hexagonal, the schema exists, a workspace can be registered end
to end, and every fake and contract suite the stories need is in place and passing. The contract
suites are green because Rust forced the `WorkspaceCache` impl to be whole; what remains for the
stories is the *behaviour* on top of it — the caching rules, the remote adapter, the interface.

---

## Phase 3: User Story 1 — Open a repository too large to fetch (Priority: P1) 🎯 MVP

**Goal**: The tree appears immediately and nothing the developer has not looked at is ever
transferred.

**Independent Test**: Open a workspace with a deep, wide tree and confirm the number of directory
listings requested equals the number of folders actually expanded, not the number that exist.

### Tests for User Story 1

- [ ] T037 [P] [US1] Write `client/core/tests/workspace_tree.rs` covering US1 scenarios 1–4 against a recording fake transport: opening fetches exactly one listing; an unexpanded folder has fetched none; collapse-and-re-expand fetches none; expanding ten folders of a hundred-thousand-entry tree fetches ten. **Count calls at the fake, never grep a log** — a log-shape change would silently pass a test that greps
- [ ] T038 [P] [US1] Write `engine/tests/read_directory.rs` against a real temp tree: shallow listing only, the contractual `(type DESC, name ASC)` ordering, paging at the 1000 limit, cursor resumption after the last name, and that a page request never recurses
- [ ] T039 [P] [US1] Write `tests/e2e/workspace-tree.spec.ts` driving the real interface: open a workspace, expand folders, assert the tree renders and captures land in `reports/screenshots/${OS}/F003/`

### Implementation for User Story 1

- [ ] T040 [US1] Implement the `ReadDirectory` use case in `engine/src/application/use_cases/workspace.rs`: resolve through `ResolvedPath`, list immediate children only, sort `(type DESC, name ASC)`, page at `limit` (default and max 1000), and return `nextCursor` exactly when more follow
- [ ] T041 [US1] Wire `workspace/readDirectory` into `engine/src/adapters/inbound/rpc.rs`, mapping refusals to `-32002`, missing paths to `-32003` and unknown workspaces to `-32001`
- [ ] T042 [US1] Implement `RemoteWorkspaceProvider::read_directory` in `client/core/src/adapters/outbound/remote_workspace.rs`. **One provider call issues at most one protocol request** (R1) — no fan-out, no retry, no prefetch, which is what makes SC-002 measure what it claims to
- [ ] T043 [US1] Implement `list_children` and `put_listing` in `client/core/src/adapters/outbound/sqlite/mod.rs`. `put_listing` replaces a parent's children atomically and preserves `file_id` for entries that remain **under the same name**, so their cached content survives with them. **It must not claim to handle renames** — a re-listing sees one name gone and another present, with no identity linking them, so the vanished entry's content cascades away and the file is re-cached on next open (FR-022a). The entry stays listed throughout (FR-022b). See [contracts/cache.md](./contracts/cache.md) C9
- [ ] T044 [US1] Implement `CachedWorkspace::read_directory` in `client/core/src/application/use_cases/cached_workspace.rs`: consult the projection first, issue exactly one shallow request on a miss, persist, return (FR-014, FR-015, FR-016, §10.1)
- [ ] T045 [US1] Add the workspace tree commands to `client/core/src/adapters/inbound/tauri_commands.rs` and the tree store in `client/ui/lib/workspace/tree.svelte.ts`
- [ ] T046 [US1] Implement `client/ui/lib/workspace/FileTree.svelte` using design system tokens and Phosphor icons only. **Make it keyboard-operable**: focusable, showing the design system's 2px accent `:focus-visible` ring and never the browser default, with Enter or Space expanding and collapsing a folder (FR-040, US1.5). Arrow-key traversal and type-ahead are out of scope and land with F006 (FR-040a) — do not build them here. No raw hex, no raw pixel values, no hard-coded font family — `npm run lint:ds` must pass (Principle I)
- [ ] T047 [US1] Add the two §1.4 sidebar measurements to `tests/perf/` : cached expand p99 under 1 ms, uncached under 250 ms, over at least 100 samples, measured at the interface/transport boundary with harness delay excluded, **printing the measured values** (A-NFR, SC-004c)

**Checkpoint**: A developer can open a large repository and browse it, and the cost is provably
proportional to what they opened.

---

## Phase 4: User Story 2 — Reopen a file without waiting for the network (Priority: P1)

**Goal**: A reopened file appears from the local projection, and the developer is never shown
content the engine would disagree with.

**Independent Test**: Open a file, change it remotely, reopen it and confirm the new content is
served — then reopen an unchanged file and confirm nothing was transferred.

### Tests for User Story 2 (cache validity is fail-first per Principle VII)

- [ ] T048 [P] [US2] Write `client/core/tests/cache_validity.rs` **before** the implementation, covering US2 scenarios 1, 2 and 5: a matching hash transfers zero content bytes; a changed hash refetches and replaces; **a file marked `MODIFIED` in git with unchanged content is still served from the cache**. Confirm they fail first
- [ ] T049 [P] [US2] Write `client/core/tests/cache_verification.rs`: `Verifying` is published before the stat is issued and stays published until it resolves (US2.3); no bytes reach the caller before confirmation across every ordering the fake can produce (SC-004a); a fake engine that never answers ends the wait at the limit and yields `Unverified` (US2.4, SC-004b). Use `FakeClock` — **a test that really sleeps for two seconds is a test nobody runs on every commit**
- [ ] T050 [P] [US2] Write `engine/tests/read_file.rs` against a real tree: ranged reads, a range past EOF returning zero bytes with the correct `totalSize`, a `length` above 512 KiB refused with `-32602` rather than truncated, and binary content surviving byte-for-byte. **Add the stat-then-read race**: take a `stat`, rewrite the file on disk, then read it, and assert the returned `sha256` differs from the one `stat` gave — so a caller assembling ranges can detect that the file moved underneath it. Without this assertion FR-021's internal-consistency claim is unverifiable, and an implementation that reported a cached hash beside fresh bytes would pass every other test in this file
- [ ] T051 [P] [US2] Write `tests/e2e/cache-verification.spec.ts` asserting the verification indicator is visible while a confirmation is outstanding (FR-021b)

### Implementation for User Story 2

- [ ] T052 [US2] Implement the `Stat` and `ReadFile` use cases in `engine/src/application/use_cases/workspace.rs`: `stat` hashes the file with `sha2` and omits `sha256` for a directory; `read_file` serves ranges and refuses anything above 512 KiB
- [ ] T053 [US2] Wire `workspace/stat` and `workspace/readFile` into `engine/src/adapters/inbound/rpc.rs` with base64 content and `encoding: "base64"` always — there is no utf8 path (FR-003)
- [ ] T054 [US2] Implement `RemoteWorkspaceProvider::stat` and `::read_file` in `client/core/src/adapters/outbound/remote_workspace.rs`, routing reads above the threshold to `BulkTransfer` rather than chunking them through the channel (FR-025, A-BULK, §4.6)
- [ ] T055 [US2] Implement the bulk adapter in `client/core/src/adapters/outbound/bulk/mod.rs` as a second `ssh` invocation on the existing control master. **It must pass `ControlMaster=no`** — F002 learned that a bulk invocation which becomes the master backgrounds itself holding the inherited stdout pipe and then waits forever for an EOF that cannot arrive
- [ ] T056 [US2] Implement `lookup` and `put_content` in `client/core/src/adapters/outbound/sqlite/mod.rs`: the §5.4 join for lookup; for writes, **hash first and compress second** (§5.6, C1), refuse content above the 8 MiB cap with `StoreOutcome::NotEligible`, and set `is_cached` and `last_accessed_at` in the same transaction as the blob (invariants 3 and 9)
- [ ] T057 [US2] Implement `touch` in `client/core/src/adapters/outbound/sqlite/mod.rs`, recording the access in the same transaction that reads the entry (FR-028)
- [ ] T058 [US2] Implement `CachedWorkspace::read_file` for the connected path in `client/core/src/application/use_cases/cached_workspace.rs`, in the order [design.md](./design.md) specifies: publish `Verifying`, then stat with the 2 s limit, then branch on match, mismatch or expiry. **Nothing reaches the caller before the branch resolves** (FR-021a). A `StoreOutcome::Failed` is logged and discarded, never propagated (FR-034)
- [ ] T059 [US2] Implement `CachedWorkspace::stat` in `client/core/src/application/use_cases/cached_workspace.rs` with pass-through when connected
- [ ] T060 [US2] Add the `Verifying`, `Current` and `Unverified` states to `client/ui/lib/statusbar/presentation.ts`, **each carrying a non-colour affordance** — a Phosphor glyph, a label, or both — so the state survives a greyscale render (FR-039) and implement `client/ui/lib/workspace/VerifyBadge.svelte`. Extend the **existing** state map rather than adding a parallel one — F001 shipped a bug where a test kept its own copy of that map and passed while the component crashed

- [ ] T091 [US2] Write the rename test in `client/core/tests/cache_validity.rs`: cache a file, call `WorkspaceCache::rename` to move it, then look it up under the new path and assert **the same `file_id` and the same content blob**, with zero bytes transferred (FR-022, US2 scenario 7). Then assert the other half: run `put_listing` over a folder where a cached name vanished and a new one appeared, and confirm the old content is gone, the new entry is listed, and nothing errored (FR-022a, FR-022b, US2 scenario 8). **Both halves matter** — the first proves opaque `file_id` works, the second proves nothing pretends it works where it cannot
- [ ] T092 [US2] Implement `rename` in `client/core/src/adapters/outbound/sqlite/mod.rs`: update `relative_path`, `parent_path` and `name` for one `file_id` and **never touch `file_contents`** (FR-022, C9). This is the operation A-B5's opaque `file_id` exists for. Its first caller is F006's write path — document that on the function, so nobody wires `put_listing` into it on the assumption that a re-listing knows what moved

**Checkpoint**: Cached files open without transfer, changed files refetch, a wedged engine
produces a stated outcome rather than a frozen window, and a known rename keeps its content while
an observed one honestly does not.

---

## Phase 5: User Story 3 — Address a workspace unambiguously (Priority: P1)

**Goal**: Two checkouts of the same repository are distinct workspaces; neither shadows the other.

**Independent Test**: Register two workspaces whose display names collide, populate both, and
confirm each reads back its own content.

### Tests for User Story 3

- [ ] T061 [P] [US3] Write `client/core/tests/workspace_registry.rs` against a real database file: two workspaces with identical display names each read back their own content with zero cross-reads (US3.1, FR-010, SC-007); re-opening attaches rather than duplicating (FR-011, US3.2); deleting removes content **and** tree (FR-012, US3.3); and **the projection survives a restart** — close the cache, reopen it from the same path, and confirm the tree and content are still there (FR-017). The last is the only assertion that a persisted store is actually persisted, and nothing else in the suite would fail if it were opened in memory
- [ ] T062 [P] [US3] Write `engine/tests/register.rs`: re-registering the same id against the same path is idempotent; against a different path is an error; a path that is not a directory or is unreadable is refused at registration, naming the workspace rather than a file inside it

### Implementation for User Story 3

- [ ] T063 [US3] Complete `RegisterWorkspace` in `client/core/src/application/use_cases/register_workspace.rs` with the attach-versus-create decision keyed on `WorkspaceId` and nothing else. **Nothing may key on the display name** (FR-010, A-WORKSPACE)
- [ ] T064 [US3] Implement workspace deletion in `client/core/src/application/use_cases/register_workspace.rs` and `forget` in the SQLite adapter, relying on `ON DELETE CASCADE` for content removal (FR-012). Assert in the test that the cascade actually fired, since it depends on the per-connection `foreign_keys` pragma from T027
- [ ] T065 [US3] Add workspace registration and deletion commands to `client/core/src/adapters/inbound/tauri_commands.rs`
- [ ] T093 [US3] Write the vanished-root test in `client/core/tests/workspace_registry.rs` and `engine/tests/register.rs`: register a workspace, delete its root on disk, then issue a read. Assert the engine reports **the workspace as gone**, distinctly from a path missing inside it, and that the client stops presenting the projection as a live view (FR-038, SC-015). **Assert the specific code** — `-32009`, not `-32001` and not `-32003`. Asserting only that the read failed would pass for an implementation that reports every deleted root as an ordinary not-found; asserting `-32001` would pass for one that sends the client into a re-registration loop. Both are the confusion this requirement exists to prevent
- [ ] T094 [US3] Implement the distinction in `engine/src/application/use_cases/workspace.rs` and `engine/src/domain/path.rs`: when resolution fails because the **registered root itself** no longer resolves, return `WorkspaceGone` (`-32009`) rather than a missing path (`-32003`) or an unregistered workspace (`-32001`). The root is canonicalised once at registration, so this is a re-check of the root before the per-request prefix comparison, not a second canonicalisation of every path
- [ ] T095 [US3] Surface it: add the `Gone` state — with a non-colour affordance (FR-039) — to `client/ui/lib/statusbar/presentation.ts` alongside the other five, map `ProviderError::WorkspaceGone` to it in `client/core/src/application/use_cases/cached_workspace.rs`, and render it in `client/ui/lib/workspace/tree.svelte.ts` so the projection is no longer presented as current. **`Gone` is a variant, not a flag on `Unavailable`** — offline-and-uncached stops being true when the connection returns, and a deleted workspace does not. **This is not the offline path** — offline means possibly stale and still true; gone means the thing being projected does not exist, which is the one case where the cache stops being a stale fact and becomes fiction

**Checkpoint**: Two checkouts of one repository coexist, each with its own projection, and a
workspace that has been deleted underneath the client says so.

---

## Phase 6: User Story 4 — Keep the cache healthy across time (Priority: P2)

**Goal**: Disk use stays bounded, trees survive eviction, and upgrading costs neither the cached
content nor a window that appears to have hung.

**Independent Test**: Age content past the retention window, run eviction, and confirm blobs are
gone while the tree is intact and files are marked uncached.

### Tests for User Story 4

- [ ] T066 [P] [US4] Write the retention half of `client/core/tests/cache_maintenance.rs` against a real database file and `FakeClock`: content aged past fourteen days is removed while **every** `files` row survives (US4.1, US4.2, SC-008); every cache hit records its access so retention measures use rather than age (US4.4, FR-028); zero evictions occur while a workspace is open (US4.5, SC-008a); an evicted file re-opens with no error surfaced (US4.3, FR-029)
- [ ] T067 [US4] Write the migration half of `client/core/tests/cache_maintenance.rs`: a v0 database migrates to v1 with content preserved (US4.6, SC-013); a migration **killed mid-step** — open, begin a step, drop the connection without committing — leaves the old version intact and readable (FR-018c); a deterministically failing migration discards and rebuilds and says so (US4.7, SC-013b); a `user_version` newer than this build takes the same discard path
- [ ] T068 [US4] Write the progress assertion in `client/core/tests/cache_maintenance.rs`: **count the reports and measure the gaps between them**, asserting at least one per second for the whole duration (SC-013a). Asserting that *a* progress message was sent passes for an upgrade that then hangs silently — that exact weakness was caught twice during specification
- [ ] T069 [P] [US4] Write `tests/e2e/cache-maintenance.spec.ts` asserting the migration state is visible during an upgrade (FR-018a)

### Implementation for User Story 4

- [ ] T070 [US4] Implement `evict` in `client/core/src/adapters/outbound/sqlite/mod.rs`: delete from `file_contents` only, clear `is_cached`, and **never** remove a `files` row (FR-027, §5.5, C4)
- [ ] T071 [US4] Implement `MaintainCache` in `client/core/src/application/use_cases/maintain_cache.rs`: migrate, then evict, once, publishing `MaintenancePhase` at least once per second throughout. `run()` **never returns an error** — a failure becomes a rebuild and is reported (FR-018b)
- [ ] T072 [US4] Complete the migration ladder in `client/core/src/adapters/outbound/sqlite/migrate.rs` with the discard-and-rebuild path and the developer-facing message that cached content was rebuilt
- [ ] T073 [US4] Add the three rendered maintenance states — `Migrating`, `Rebuilding` and `Evicting` — each with a non-colour affordance (FR-039) — to `client/ui/lib/statusbar/presentation.ts` and implement `client/ui/lib/workspace/MaintenanceBanner.svelte`. Use the names [data-model.md](./data-model.md) defines and no others; `Idle`, `Checking` and `Ready` are not rendered. **Do not collapse them into one "maintaining" state** — FR-018a requires a state saying a *migration* is running and SC-013a asserts on migration reports specifically, so a merged state makes that criterion unmeasurable
- [ ] T074 [US4] Verify in `client/core/src/composition.rs` that maintenance completes before any provider is constructed, and that the guard added in T035 actually prevents the wrong order rather than documenting it

**Checkpoint**: A months-old installation across several releases still works, and an upgrade shows
its progress.

---

## Phase 7: User Story 5 — Find a file by path while disconnected (Priority: P2)

**Goal**: An outage is an inconvenience rather than a stop.

**Independent Test**: Populate a cache, disconnect, search by path fragment, and confirm results
come from the projection with no request attempted.

### Tests for User Story 5

- [ ] T075 [P] [US5] Write `client/core/tests/cache_offline.rs` with the connection source set to disconnected: path search returns results with **zero requests attempted, counted at the transport fake** (US5.1, SC-011) — asserting only that the search succeeded would pass for an implementation that tried the network, timed out and fell back; a cached file is served marked possibly stale (US5.2, FR-032); an uncached file produces a stated reason rather than an empty document (US5.3, SC-012)
- [ ] T076 [P] [US5] Write `client/core/tests/fts_sync.rs` asserting the trigger-maintained index stays in step: insert, rename and delete a `files` row and confirm `files_fts` matches after each. **This is the test that would have caught the gap in §5.2** — without the triggers it returns nothing, fast, with no error

### Implementation for User Story 5

- [ ] T077 [US5] Implement `search_paths` in `client/core/src/adapters/outbound/sqlite/mod.rs` against `files_fts`, never a leading-wildcard `LIKE` (§5.2, §5.4)
- [ ] T078 [US5] Implement `SearchPaths` in `client/core/src/application/use_cases/search_paths.rs`, which never touches a provider (C6)
- [ ] T079 [US5] Implement the disconnected branch of `CachedWorkspace::read_file` in `client/core/src/application/use_cases/cached_workspace.rs`. **Consult `ConnectionStatusSource` before the cache, not after a failure** — an outage must cost nothing and produce no timeout
- [ ] T080 [US5] Add the `PossiblyStale` and `Unavailable` states, each with a non-colour affordance (FR-039), to `client/ui/lib/statusbar/presentation.ts`, and surface offline search in `client/ui/lib/workspace/tree.svelte.ts`

**Checkpoint**: A developer on a plane can find and read what they have already visited.

---

## Phase 8: Polish & Cross-Cutting Concerns

- [ ] T081 Implement the SC-010 compression gate in `tests/perf/`: cached content occupies at most half the disk of what it represents, measured over **this repository** rather than generated text, with the achieved ratio printed (A-NFR). Generated text compresses far better than code and would make the budget meaningless
- [ ] T082 [P] Add the opt-in real-`sshd` test in `client/core/tests/workspace_real_sshd.rs`, gated on `APEX_REAL_SSHD`, proving a bulk read attaches to the existing control master and does not become one. Excluded from the default suite because SC-014 forbids requiring a host
- [ ] T083 [P] Add the digest agreement test asserting `client/core/build.rs`'s hand-rolled SHA-256 and `sha2` produce the same hash for the same bytes, so the two implementations cannot drift apart silently
- [ ] T084 [P] Add unit tests for the cursor logic in `engine/src/application/use_cases/workspace.rs`: a cursor past the last entry returns empty with no `nextCursor`; a cursor for a name that no longer exists resumes at the next name rather than failing
- [ ] T085 [P] Update `docs/` with the cache's location on disk, what discarding it costs, and the three constants a developer may hit — the 8 MiB cap, the 14-day window and the 2 s confirmation limit
- [ ] T086 Verify in `client/core/src/adapters/outbound/remote_workspace.rs` and `engine/src/adapters/inbound/rpc.rs` that every `Unsupported` refusal names its owning feature (F004 or F006) so a log reads as a schedule rather than a bug (FR-004), and that no unimplemented method has any side effect. Assert it in `client/core/tests/provider_contract.rs` rather than by inspection
- [ ] T087 Run `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` across `protocol/`, `engine/` and `client/core/`. **Not a formality**: F002's clippy caught a `std::thread::sleep` inside an async function, and this feature has a blocking boundary at the SQLite adapter, which is exactly the shape that mistake takes
- [ ] T097 Add the greyscale assertion to `tests/e2e/cache-verification.spec.ts` and `tests/e2e/cache-maintenance.spec.ts`: render with colour removed and assert **all eight published states remain distinguishable from one another**, zero pairs collapsing (FR-039, SC-016, US2.9, US4.8). Follow `tests/e2e/rail-greyscale.spec.ts`, which F018 already uses for exactly this — a greyscale render is the only way to prove colour is not the sole channel, and the adherence lint provably cannot: it restricts raw hex, raw pixel values and font families and has no view of state encoding
- [ ] T098 Add the keyboard assertion to `tests/e2e/workspace-tree.spec.ts`: tab to the tree, assert the focus indicator is the design system's 2px accent ring and **not** the browser default, and expand and collapse a folder with Enter or Space (FR-040, SC-017, US1.5). Follow `tests/e2e/rail-keyboard.spec.ts` from F001
- [ ] T088 Run `npm run lint:ds` and `npm run gate:fidelity` against the three new interface surfaces — `client/ui/lib/workspace/FileTree.svelte`, `VerifyBadge.svelte` and `MaintenanceBanner.svelte` — confirming tokens only, no raw hex, no raw pixel values (Principle I)
- [ ] T089 Execute [quickstart.md](./quickstart.md) end to end and record the results in it, including the three printed performance numbers. **Read the test summary line, not the exit code** — F001 found a gate reporting `failed=no` while six tests failed, because `test result: FAILED.` puts the word in the third field
- [ ] T090 Tick F003's six subfeature boxes in `specs/features-map.md` **only** for what shipped, and reconcile the "local and remote implementations" line with plan.md's Structure Decision — the local provider belongs to F015 and is deliberately not built here. Do not tick a box for work that did not happen


---

## Requirement Coverage

Every functional requirement and success criterion in [spec.md](./spec.md) maps to at least one
task. This table exists so coverage is checkable rather than asserted — it is the thing
`/speckit-analyze` reads, and building it found one gap: FR-017 had no test until T061 gained one.

| Requirement | Tasks | Requirement | Tasks |
|---|---|---|---|
| FR-001 | T015, T033 | FR-022 | T091, T092 |
| FR-002 | T015, T040, T052 | FR-023 | T050, T052, T054 |
| FR-003 | T013, T053 | FR-024 | T002, T038, T040 |
| FR-004 | T015, T086 | FR-025 | T054, T055 |
| FR-005 | T019, T020 | FR-036 | T047 |
| FR-006 | T019, T020 | FR-037 | T085 |
| FR-007 | T019, T020 | FR-026 | T066, T070, T071 |
| FR-008 | T012, T019 | FR-026a | T066, T071, T074 |
| FR-009 | T011, T063 | FR-026b | T085 |
| FR-010 | T061, T063 | FR-027 | T066, T070 |
| FR-011 | T036, T061, T063 | FR-028 | T057 |
| FR-012 | T064 | FR-029 | T066 |
| FR-013 | T058 | FR-030 | T056 |
| FR-014 | T037, T044 | FR-031 | T075, T077, T078 |
| FR-015 | T037, T044 | FR-032 | T075, T079, T080 |
| FR-016 | T037, T044 | FR-033 | T075, T079, T080 |
| FR-017 | T061 | FR-034 | T016, T031, T058 |
| FR-018 | T028, T067, T072 | FR-035 | T082, T089, and the default suite throughout |
| FR-018a | T068, T071, T073 | | |
| FR-018b | T067, T071, T072 | | |
| FR-018c | T035, T067, T074 | | |
| FR-019 | T014, T048, T058 | | |
| FR-020 | T014, T048 | | |
| FR-021 | T013, T050 | | |
| FR-021a | T049, T058 | | |
| FR-021b | T049, T051, T058, T060 | | |
| FR-021c | T049, T058, T060 | | |
| FR-022a | T043, T091 | | |
| FR-022b | T043, T091 | | |
| FR-038 | T093, T094, T095 | | |
| FR-039 | T060, T073, T080, T095, T097 | | |
| FR-040 | T046, T098 | | |
| FR-040a | T046 | | |

| Criterion | Tasks | Criterion | Tasks |
|---|---|---|---|
| SC-001 | T037 | SC-008 | T066 |
| SC-002 | T037 | SC-008a | T066 |
| SC-003 | T048 | SC-009 | T066 |
| SC-004 | T048 | SC-010 | T081 |
| SC-004a | T049 | SC-011 | T075 |
| SC-004b | T049 | SC-012 | T075 |
| SC-004c | T047 | SC-013 | T067 |
| SC-005 | T048 | SC-013a | T068 |
| SC-006 | T019 | SC-013b | T067 |
| SC-007 | T061 | SC-014 | T082, T089 |
| SC-015 | T093 | | |
| SC-016 | T097 | | |
| SC-017 | T098 | | |

Three requirements are satisfied by a **record** rather than by code, and their tasks write that
record: FR-037 (the cache does not make opening faster while online) and FR-026b (a session that
never restarts never evicts) are accepted limits that T085 documents, and FR-013's "the engine
wins" is enforced by T058 having no branch in which a cached hash overrides the engine's.

---

## Dependencies & Execution Order

### Phase dependencies

- **Phase 1 (Setup)** — no dependencies. T001–T003 amend the system specification and block all
  code that depends on those shapes; T004–T006 are Principle III records owed before the code
  implementing those decisions
- **Phase 2 (Foundational)** — depends on Phase 1. **Blocks every user story**
- **Phases 3–7 (User Stories)** — all depend on Phase 2. Independent of each other thereafter
- **Phase 8 (Polish)** — depends on the stories being complete

### Story dependencies

| Story | Depends on | Note |
|---|---|---|
| US1 (P1) | Phase 2 | Registration is in Phase 2 precisely so US1 does not depend on US3 |
| US2 (P1) | Phase 2 | Independent of US1 — it reads files, not trees |
| US3 (P1) | Phase 2 | Semantics of identity; the mechanism is already in Phase 2 |
| US4 (P2) | Phase 2 | Eviction needs content to evict, which US2 produces, but the tests create rows directly |
| US5 (P2) | Phase 2 | Needs a populated projection, which its tests build directly |

**T091–T098 are appended IDs** placed in their own phases: T096 in Setup (the fourth system-spec
amendment), T091–T092 in US2 (rename), T093–T095 in US3 (the vanished root), T097–T098 in Polish
(the design-system gates the constitution requires). They came from two
`/speckit-analyze` passes after numbering was fixed. Renumbering ninety-odd tasks to insert six
would have invalidated every cross-reference in this file; F002 set the precedent with its T076.

### Within a story

Tests before implementation where the constitution names fail-first — **T019 before T020** (path
containment), **T048 before T052–T058** (cache validity) and **T091 before T092** (also cache
validity — whether content survives a move is exactly that). Then: engine use case → protocol
wiring → client adapter → application rules → interface.

### Parallel opportunities

- T004, T005, T006 — three independent Appendix A entries
- T010, T011, T014–T018 — wire constants, domain types and ports, all different files.
  **T012 and T013 are excluded**: they extend `domain/workspace.rs` alongside T011 and run after it
- T021, T022 — engine ports and adapters, once T020 exists
- T030, T031, T032 — the three fakes
- T037, T038, T039 (US1) · T049, T050, T051 (US2) · T061, T062 (US3) · T075, T076 (US5)
- Once Phase 2 completes, all five stories can proceed in parallel

Tests within a story are **not** universally parallel, and the exceptions are the point: T091
shares `cache_validity.rs` with T048, T093 shares two files with T061 and T062, and T067 and T068
share `cache_maintenance.rs` with T066. Each runs after the task it shares a file with.

---

## Parallel Example: Phase 2 ports and domain

```bash
# All different files, no shared dependencies:
Task: "Implement WorkspaceId, Workspace, Location, FileId, Sha256 in client/core/src/domain/workspace.rs"
Task: "Implement CacheEntry, Validity, Presentation, MaintenancePhase in client/core/src/domain/cache.rs"
Task: "Define WorkspaceProvider in client/core/src/application/ports/workspace_provider.rs"
Task: "Define WorkspaceCache in client/core/src/application/ports/workspace_cache.rs"
Task: "Define BulkTransfer in client/core/src/application/ports/bulk_transfer.rs"
Task: "Define Clock in client/core/src/application/ports/clock.rs"
```

---

## Implementation Strategy

### MVP — User Story 1 only

1. Phase 1 (T001–T008 **and T096**): the specification is whole and the dependencies are in. T096 is an appended ID that lives inside this phase, not after it — a range written as T001–T008 would skip the §4.4 error code that T094 and T095 depend on
2. Phase 2 (T009–T036): both binaries hexagonal, schema exists, registration works
3. Phase 3 (T037–T047): lazy tree loading
4. **STOP and VALIDATE**: expand ten folders of a hundred-thousand-file tree and confirm ten
   listings were requested
5. That is a demonstrable product — a browsable remote repository of any size

### Incremental delivery

MVP → **US2** (files open from cache, verified) → **US3** (two checkouts coexist) → **US4** (the
cache survives months and upgrades) → **US5** (offline). Each adds value without breaking what came
before.

Phase 2 is unusually large for this feature — 28 of 96 tasks — because it carries the engine's
restructure and the schema. That is front-loaded cost with no user-visible result, and it is worth
knowing before starting rather than discovering at task fifteen.

---

## Notes

- `[P]` means different files and no dependency on an incomplete task. **Checked mechanically**: no
  two `[P]` tasks name the same file. Five groups violated this before the second analysis pass —
  T011–T013, T066–T068 and three pairs introduced by the first remediation — and the markers, not
  the tasks, were wrong
- Commit after each task or logical group
- **Two tests must fail before their implementation exists**: T019 (path containment) and T048
  (cache validity). Both are named in Principle VII as cases where a wrong answer is expensive
- Watch for the recurring failure in this project: **green for the wrong reason.** Eight instances
  were found across F001 and F002 — a vacuous redaction check, a priority test whose helper returned
  a constant, a gate that read the wrong awk field, a stale binary that made three broken harnesses
  pass. T037, T068, T075 and T076 each carry a note about the specific way they could pass while
  proving nothing
