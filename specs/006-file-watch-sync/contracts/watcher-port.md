# Contract: `FileWatcher` and `Clock`

**Feature**: F004 file-watch-sync | **Date**: 2026-09-24

Two outbound ports the engine acquires. Both are **capabilities, not technologies**
(Principle VIII): `FileWatcher`, not `InotifyWatcher`; `Clock`, not `Instant::now`.

This document states signatures and the guarantees a signature cannot carry. There are no bodies
here and none are implied — the adapter is `engine/src/adapters/outbound/inotify_watcher.rs` and
nothing else.

Rationale for the shapes below is in research.md, *A watcher in a runtime-free engine* and
*Watch scope: what is actually watched*. It is not restated.

**Synchronous, and that is not an accident.** `engine/Cargo.toml` records the constraint in its
own comment: the engine "stays synchronous and runtime-free, because it is embedded in the client
and transferred on every first connect". A watcher is not a reason to reverse a decision made
about binary size and transfer cost. There is one descriptor and one timer; `std::thread` plus a
poll with a timeout does the whole job, and the stdlib rung of the ladder is the right one to
stop at. **No `async`, no `tokio`, anywhere in this contract.** The precedent is
`application/ports/file_system.rs`, which is synchronous for the same reason and says so.

Rust MSRV is **1.75**, declared identically by `protocol`, `engine` and `client/core`. Nothing
below uses a language or stdlib feature newer than that.

---

## The dividing line

Plan.md states the structural rule and it is the reason both ports exist: **`inotify` appears in
exactly one file, and everything that decides anything is testable without a filesystem.** That
is what makes FR-012, FR-015 and SC-005 unit tests rather than integration tests with a sleep in
them.

| Behind the port — `inotify_watcher.rs` | In pure application code — `coalescer.rs`, `use_cases/watch.rs` |
|---|---|
| Owning the inotify file descriptor | The 100 ms per-path window and its trailing-edge flush (FR-012) |
| `add_watch` / `rm_watch`, and the descriptor-to-path table | Pairing `MovedFrom` with `MovedTo` by cookie (FR-011) |
| Decoding kernel event records into `RawEvent` | Classifying an unpaired half as `deleted` or `created` |
| Blocking on the descriptor with a timeout | Counting 256 distinct paths in a rolling second (FR-015) |
| Translating `ENOSPC` into `WatchRefusal::CapacityExhausted` | Filtering excluded paths (FR-008) |
| Nothing else | The requested-set delivery filter (file-events.md, *What is delivered*) |
| | Deriving the host watch set — ancestors, tab parents, the root — from the requested set |
| | Building a wire `FileEvent` from a `RawEvent` and its watch's path |

Two entries are worth naming because they look like adapter work and are not.

**Exclusion is never consulted behind the port** (FR-008, FR-006, FR-007). The resolved set lives
on the registered workspace and is read by whatever needs it (research.md, *Where the exclusion
set lives*). It is consulted twice in application code — when a watch is requested, producing the
`excluded` refusal of watch-methods.md, and when an event is classified, dropping it before
coalescing. Putting either inside the adapter would make SC-002 and SC-003 need a filesystem.

**The requested set is never known behind the port.** The adapter is told to watch a directory; it
is never told why, and it never decides whether an observed event is delivered. That decision
needs the requested set, and the requested set is what the use case holds.

---

## `FileWatcher`

`engine/src/application/ports/file_watcher.rs`. The event types it yields live in
`engine/src/domain/watch.rs`, per the plan's source layout.

```rust
use crate::domain::path::ResolvedPath;
use crate::domain::watch::RawEvent;
use std::time::Duration;

/// Why the host would not watch a path.
///
/// Every variant is a refusal the caller turns into a `{path, reason}` entry in
/// `workspace/watch`'s result (FR-005). **None of them is a panic and none of them fails the
/// call**: FR-005a requires the workspace to open and browse when watching is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchRefusal {
    /// The host's per-user watch limit is reached — `ENOSPC` from `inotify_add_watch`, which
    /// does not mean a full disk. The resource is limited per user rather than per workspace,
    /// so another workspace can exhaust this one.
    CapacityExhausted,
    /// The path was resolved and is gone by the time the watch is added. A race, not a bug:
    /// the client asked about a folder that has since been deleted on the host.
    NotFound,
    /// Inside the root, present, and not a directory. The engine watches directories only.
    NotADirectory,
    /// The descriptor itself failed. `ErrorKind` rather than `io::Error` so the enum stays
    /// `PartialEq` and a test can assert on the refusal it provoked.
    Unavailable(std::io::ErrorKind),
}

pub trait FileWatcher: Send {
    /// Begin observing one directory's immediate children.
    ///
    /// Takes a `ResolvedPath`, which has no public constructor other than `resolve`, so this
    /// port cannot be handed a path whose containment was never checked (§4.7, Principle VI).
    /// Idempotent at the host: adding a watch for a directory already watched returns the same
    /// `WatchId`, which is inotify's own behaviour and is why the requested set can be a set.
    fn watch_directory(&mut self, path: &ResolvedPath) -> Result<WatchId, WatchRefusal>;

    /// Stop observing. Removing a `WatchId` that is already gone succeeds.
    fn unwatch(&mut self, watch: WatchId) -> Result<(), WatchRefusal>;

    /// Block for at most `timeout`, then append everything that arrived to `out`.
    ///
    /// The timeout is the time remaining until the nearest pending coalescing window is due, so
    /// the thread wakes to flush and for nothing else. Returning into a caller-owned buffer
    /// rather than a fresh `Vec` means a quiet workspace allocates nothing per poll.
    ///
    /// A timeout that expires with nothing to report is `Ok(())` with `out` unchanged — not an
    /// error, and not a condition any caller branches on.
    fn poll(&mut self, timeout: Duration, out: &mut Vec<RawEvent>) -> Result<(), WatchRefusal>;

    /// How many watches this instance currently holds.
    ///
    /// Not diagnostics: SC-009, SC-009a and SC-009b are assertions about host watch resources,
    /// and a count the port will not report is a count a test measures by reading `/proc`.
    fn held(&self) -> usize;
}
```

### What the port yields

```rust
/// A host watch descriptor. Opaque: the caller maps it back to a path through its own table,
/// and nothing outside the adapter may assume it is inotify's `i32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatchId(u32);

/// inotify's rename pairing key. The two halves of a move within the workspace carry the same
/// value; nothing else does (research.md, *Rename detection*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenameCookie(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    Created,
    Modified,
    Deleted,
    /// One half of a move. Paired with `MovedTo` by cookie inside the coalescing window; an
    /// unpaired one at flush time is a `deleted` (FR-011).
    MovedFrom,
    /// The other half. Unpaired, it is a `created`.
    MovedTo,
    /// The watched directory itself was deleted or moved. On the workspace root this is the
    /// `-32009` condition — registered, and the thing it pointed at is gone (§4.4).
    WatchDropped,
    /// The kernel's event queue overflowed and events were lost. See *Overflow* below.
    Overflow,
}

/// One kernel event, before anything decides what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    /// Which watch observed it. The caller resolves this to a directory.
    pub watch: WatchId,
    /// The child's name within that directory. `None` for `WatchDropped` and `Overflow`, which
    /// concern the watch itself rather than anything inside it.
    pub name: Option<String>,
    pub kind: RawKind,
    /// Present for `MovedFrom` and `MovedTo`, absent otherwise.
    pub cookie: Option<RenameCookie>,
}
```

`name` is `String` rather than `OsString`, following `RawEntry` in `ports/file_system.rs`. A
filename that is not valid UTF-8 is **dropped at the adapter with a logged reason**, never
lossily converted: a lossy conversion produces a path that does not exist, which the client would
then act on — and FR-014 makes the client refuse paths, not repair them.

### Guarantees

| # | Guarantee | Requirement |
|---|---|---|
| W1 | Capacity exhaustion is a returned refusal. The port never panics, never aborts, and never silently succeeds without a watch | FR-005, FR-005a, SC-009c |
| W2 | A refusal for one directory affects no other. The caller keeps every watch it already holds and adds every one it can | FR-005a, FR-027 |
| W3 | The port cannot be given an unchecked path. `ResolvedPath` has no constructor but `resolve` | §4.7, Principle VI |
| W4 | The port makes no delivery decision. It reports what the kernel said; what reaches the wire is decided in application code | FR-003, FR-008, Principle VIII |
| W5 | `poll` never blocks longer than `timeout`. A window due in 40 ms is flushed in 40 ms, whatever the workspace is doing | FR-012, SC-001 |
| W6 | The port yields no content and reads no file. There is no path from a `RawEvent` to bytes | FR-013 |
| W7 | Lost events are reported as `Overflow`, never dropped silently | FR-005, FR-025 |
| W8 | `Send` but **not** `Sync` — see below | research.md, *A watcher in a runtime-free engine* |

**W8 is a deliberate departure from the engine's existing ports.** `FileSystem` and
`WorkspaceRoots` are `Send + Sync` because `dispatch` shares them across the session loop. The
watcher and the coalescer are owned by one dedicated thread and are never shared, so requiring
`Sync` would be a promise nothing needs — and it would rule out the single-threaded interior
mutability the fakes below use. A port should promise what its callers require and no more.

### Overflow — an unrecorded case, flagged rather than assumed

`IN_Q_OVERFLOW` is real: the kernel's per-instance queue is finite, and a burst large enough
overruns it. Events are **lost**, not delayed.

`RawKind::Overflow` exists so that loss is surfaced through the port rather than appearing as
silence — which is the exact failure FR-005 and FR-025 forbid. The natural handling is the one
§10.4 already specifies for the case where the client cannot know which parts moved: emit
`workspace/invalidateAll` for the workspace, mark the tree stale, and let the developer's next
navigation resolve it. That is also FR-026's reasoning applied to a second cause.

**Neither spec.md nor research.md records this.** research.md fixes the bulk threshold and the
coalescing window and is silent on queue overflow. The handling above follows FR-015 and §10.4 by
analogy and is written here so it is a decision rather than something discovered in a burst test
— but it closes an alternative (drop the batch and continue) and therefore owes an Appendix A
record under Principle III, or an addition to A-COALESCE, before the code exists.

---

## `Clock`

`engine/src/application/ports/clock.rs`.

```rust
use std::time::Instant;

/// Time, as a capability.
///
/// Coalescing and the bulk window are time-dependent, and Principle VIII names the clock an
/// outbound port explicitly. A fake clock is what makes "a thousand writes in one second" a unit
/// test rather than a sleep (plan.md, Complexity Tracking).
pub trait Clock: Send {
    fn now(&self) -> Instant;
}
```

### Guarantees

| # | Guarantee | Requirement |
|---|---|---|
| K1 | Monotonic. `Instant`, never `SystemTime`: a window measured against a wall clock that steps backwards over NTP stops flushing, and a watcher that stops flushing is a watcher that reports nothing | FR-012, FR-025 |
| K2 | Nothing in the coalescer calls `Instant::now()` directly. The absence of that call is what makes SC-007 assertable without waiting a second | Principle VIII, SC-007 |
| K3 | `Send`, not `Send + Sync`, for W8's reason | — |

**Why `Instant` and not a bespoke monotonic newtype.** `Instant` already is the monotonic clock,
and on MSRV 1.75 there is no way to construct one from a value — but `Instant + Duration` is
stable and always has been. A fake therefore captures a base at construction and returns
`base + advanced`, which is enough to drive every window in the coalescer and costs no new type.
Inventing a `Monotonic(Duration)` newtype would buy a constructor the tests do not need.

---

## The fakes — why they are part of the contract

F003 established that an in-memory fake is a contract obligation rather than a test helper, for a
reason that applies here with more force: **the adapter is Linux-only by construction and the
client-side suite never touches it**, so a fake is the only thing that proves the port is
implementable more than once.

`FakeWatcher` must reproduce:

- **A scripted event queue.** `poll` hands out the next batch and returns; the coalescer's window
  arithmetic is exercised against exact inputs (FR-012, SC-007).
- **A capacity ceiling that can be set.** `watch_directory` refuses on demand with
  `CapacityExhausted`, so FR-005, FR-005a and SC-009c are testable on a machine with plenty of
  watches free. A fake that cannot fail tests only the happy path, and the happy path is not
  where FR-005 lives.
- **A record of every add and remove.** SC-009, SC-009a and SC-009b are assertions about which
  directories are watched, and they need the set, not a count.
- **Unpaired `MovedFrom` and `MovedTo`.** The two unpaired classifications are the cases a real
  filesystem produces rarely and a rename test must produce every run (FR-011).
- **`Overflow` and `WatchDropped` on demand.** Neither is reachable from a test that uses a real
  filesystem without extraordinary effort, and both have required behaviour.

`FakeClock` must reproduce:

- **Advance on command**, never on elapsed real time. `advance(Duration)` and nothing else.
- **A `poll` that cooperates with it.** `FakeWatcher::poll` advances `FakeClock` by its `timeout`
  when the scripted queue is empty, so a window expires because the test said a window expired.
  Without that coupling every volume test is wall-clock-dependent and slow, which is the outcome
  the `Clock` port was added to prevent.

Both live beside the tests, not in `src/`, so neither can be wired into a real composition by
accident — F003's rule for `FakeWorkspace`, for the same reason.

---

## What is NOT behind this port

**The wire.** The watcher thread serialises notifications itself and writes them under the same
mutex the responder uses (research.md, *A watcher in a runtime-free engine*). Events are
notifications, so they need no correlation with a pending request. What the thread must not do is
hold that mutex for longer than one frame: §4.6 makes this one pipe and one queue and FR-016
forbids event delivery delaying interactive traffic. **That is a measurement obligation under
Principle V, not a comment.**

**The client.** `LocalWorkspaceProvider::watch()` returns `Unsupported` in v1 and no native
watcher ships for macOS or Windows (research.md, *Local mode does not watch in this feature*,
A-WATCHLOCAL). FR-027 already specifies the resulting behaviour — browsing and reading continue
and the loss is stated — so the degradation path is a requirement with a test rather than a gap,
and local mode exercises it for free. The port is the seam that makes a future local watcher a
new adapter rather than a change.

**A second implementation for another platform.** There is one adapter, for Linux, because §10.3
puts the watches in the engine and the engine runs on Linux. The port exists for testability and
for Principle VIII, not for portability that nothing has asked for.
