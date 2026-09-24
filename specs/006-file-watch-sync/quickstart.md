# Quickstart: Validating File Watch Sync

**Branch**: `feature/F004-file-watch-sync` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

How to prove this feature works, in the order a reviewer should run it. Every scenario below maps
to a user story or a success criterion, and **none of them needs a remote host or a network**
(FR-028, SC-013).

For what each interface promises, see [contracts/watch-methods.md](./contracts/watch-methods.md),
[contracts/file-events.md](./contracts/file-events.md) and
[contracts/watcher-port.md](./contracts/watcher-port.md). For the two new columns, the
`user_version` 1 to 2 migration and the entities behind them, see
[data-model.md](./data-model.md). This document restates neither — it says how to run them and
what must be true afterwards.

**On the commands below.** Every `make` target and every `npm run` script named here exists today
— check `Makefile` and `package.json`. Every `cargo test --test <name>` names a target **this
feature adds**; none of them exists on `master`, and `tasks.md` is where they are created. If a
target below does not exist when you run it, the task that creates it has not been done. That is a
finding, not a typo to work around.

---

## Prerequisites

```bash
cargo --version            # 1.75 or newer — the MSRV all three crates declare
node --version             # 20 or newer
make setup                 # npm ci
uname -s                   # Linux, for the engine-side sections below
```

Two environment facts decide what you can run:

- **The watch adapter is Linux-only by construction.** `inotify` appears in exactly one file
  (plan.md, *Structure Decision*). On macOS and Windows the domain, application and client suites
  all run; the engine-side adapter sections are the ones that do not, and local mode's `watch()`
  returns `Unsupported` by decision (research.md, *Local mode does not watch in this feature*).
  That is a stated degradation with its own test (FR-027), not a hole.
- **`make test` builds the engine binary before running the suite** — `cargo build -p apex-engine`
  is its first line. This matters more here than it did for F003: the delivery sections spawn that
  built binary as a local child process, so a stale or missing `target/debug/apex-engine` makes
  them fail for a reason that has nothing to do with watching.

For the watch-descriptor sections, read the host's own limit first so a failure is legible:

```bash
cat /proc/sys/fs/inotify/max_user_watches
cat /proc/sys/fs/inotify/max_user_instances
```

---

## 0. What stands in for the remote host

Nothing below talks to a host. Each scenario says which double it uses, because a scenario whose
stand-in is unstated is a scenario nobody can reproduce.

| Stand-in | Replaces | Lives in | What it must not be asked to do |
|---|---|---|---|
| `FakeFileWatcher` | `inotify` and the filesystem | `engine/tests/common/` | Nothing timing-dependent. It is fed `RawEvent`s by hand, so a burst is a loop, not a sleep |
| `FakeClock` (engine `Clock` port) | Elapsed time | `engine/tests/common/`, mirroring `client/core/tests/common/fake_clock.rs` | Be replaced by `sleep`. A volume test that sleeps for a second is a test nobody runs on every commit |
| `apex-mock-daemon` | The remote `sshd` and its link | `client/core/tests/mock_daemon/` | Understand a request body. It answers framing and timing only — `delay=250,drop=20`, `stall`, `close-mid-frame` |
| The locally spawned real engine | The remote engine process | `target/debug/apex-engine`, spawned on real pipes through `ProcessSpawner` | Reach a network. It is a child process on this machine; the transport under test is the production one |
| Recording fake transport | The wire, when the question is "how many" | `client/core/tests/common/` | Be substituted by a log grep. F003's counts come from a counting fake precisely because a log-shape change silently passes a grep |
| A real temp filesystem tree | The host's working copy | `tempfile`, as `path_containment` already does | Be faked for the symlink and atomic-rename cases, which a fake cannot prove |
| `npm run stub:connection -- --state disconnected` | A dropped link, for the running shell | `scripts/stub-connection.mjs` | Appear in an automated assertion. It drives a manual look, not a gate |

**The one gap in this row of doubles, stated before you hit it.** `apex-mock-daemon` implements no
§4.8 method on purpose, and `the_mock_implements_no_engine_method` in its `main.rs` fails the build
if any §4.8 method name appears in that directory. F004 is the first feature whose traffic includes
an **unsolicited server-to-client frame**, so the mock needs a directive that emits one — and that
directive cannot carry `workspace/onFileEvent` as a literal without breaking the guard that keeps a
second engine from growing there. Resolve it in `tasks.md` as a stream-shape directive that emits a
frame the test supplies, or the delivery sections have no way through the mock at all. See
*Known gaps*.

---

## 1. The whole suite

```bash
make test
```

That is `cargo build -p apex-engine`, `cargo test --workspace`, clippy with `-D warnings`,
`cargo fmt --all --check`, `npm run test:unit`, `npm run lint` and `npm run lint:ds` — the gate as
the Makefile defines it, so nobody reconstructs it by hand and gets it subtly different.

**Read the summary line, not the exit code.** F001 found a gate reporting `failed=no` while six
tests failed, because `test result: FAILED.` puts the word in the third field and the check looked
at the second. If a number other than zero appears after `failed:`, the run failed.

---

## 2. US1 — a colleague's change arrives without being asked for

**Set up**: a workspace registered against a real temp directory (no host), the `FakeFileWatcher`
for the application-level assertions and the real adapter for the Linux-only ones, `FakeClock` for
anything that involves the 100 ms window.

```bash
cargo test -p apex-engine --test watch_scope
cargo test -p apex-engine --test watch_exclusions
cargo test -p apex-shell  --test file_event_apply
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US1.1 | Create a file in an expanded folder | The row appears in the projection's `files`; exactly one event, no re-listing of the folder |
| US1.2 | Delete a file in an expanded folder | The row goes; the blob's fate is decided by §5.3, not by the event |
| US1.3 | Change a file whose content is cached | The blob is marked unproven and **still present**; the next read confirms by hash and serves the new bytes |
| US1.4 | Change files inside a folder never expanded | Zero events delivered, zero fetches issued — counted at the recording fake |
| US1.5 | Expand a folder, then collapse it | `workspace/watch` on expand, `workspace/unwatch` on collapse, asserted on the calls the engine received, not on client-side bookkeeping |
| US1.6 | Change files inside `node_modules`, then expand it and change more | Zero events in both halves |

US1.3 is the one worth watching run. It is the scenario that fails if anybody makes an event a
second route to validity — which FR-019 forbids and research.md's *Representing unproven content*
records as a flag beside `Validity`, never a variant inside it.

---

## 3. US2 — a branch switch without a flood

**Set up**: `FakeFileWatcher` feeding a scripted burst, `FakeClock` driving the rolling
one-second bulk window, and the recording fake transport counting what reaches the wire. The
thresholds are fixed values, not judgements: **100 ms** per path, **256 distinct paths in one
second** (research.md, A-COALESCE).

```bash
cargo test -p apex-engine --test coalescer
cargo test -p apex-shell  --test invalidate_all
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US2.1 | Touch 257 distinct paths inside one second | Exactly one `workspace/invalidateAll`, zero `workspace/onFileEvent` frames |
| US2.2 | Deliver a wholesale invalidation | Tree marked stale; **zero** listing requests until a navigation is simulated |
| US2.3 | Deliver a wholesale invalidation with a populated cache | Blob count and blob bytes identical before and after |
| US2.4 | Issue an interactive read while the burst is in flight | Measured p99 printed and inside §1.4 — see §7 |

US2.3 compares **bytes, not row counts**. An implementation that cleared and refilled the content
table keeps the count identical and loses everything the developer had offline.

---

## 4. US3 — knowing the file you are reading has moved on

**Set up**: the client core with a fake tab set, plus the end-to-end spec for the visible half.
"Currently viewing" means an open tab, focused or not (FR-023).

```bash
cargo test -p apex-shell --test file_event_apply
cargo test -p apex-shell --test rename_subtree
npm run test:unit -- tests/unit/tab-marking.test.ts
npm run e2e                        # tests/e2e/file-watch.spec.ts, Linux, per A-E2E
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US3.1 | Change a file with an open tab | The tab is marked changed |
| US3.2 | Same, with no unsaved local edits | The new content is available to show; nothing is written underneath the developer |
| US3.3 | Change a file whose tab is open but unfocused | That tab is marked; the focused tab's identity is unchanged — asserted on the focused tab id before and after |
| US3.4 | Change a file with no open tab | Zero prompts, zero dialogs, zero focus changes; a tree row updating in place is not an interruption |
| US3.5 | Change a file opened through search whose folder is collapsed | Still reported — this is FR-003c, and it fails for exactly the file a developer is reading if the watched set is only the expanded folders |
| US3.6 | Close the last tab on a file, then change it | Zero reports, **and** an `unwatch` covering it if no expanded folder still needs it |
| US3.7 | Rename a file on the host | One `renamed` event naming both paths; the tree entry moves; the cached blob survives |
| US3.8 | Rename a directory with a subtree beneath it | One event; every descendant row rewritten in **one transaction**; a sibling directory sharing the prefix untouched |

US3.6 asserts two things on purpose. A test that only counts reports passes for an implementation
that keeps the watch open on the host and drops the report at the client — correct on screen and
leaking the scarce resource FR-004 exists to return.

US3.8's sibling case is the one research.md singles out: matching `src%` also rewrites
`src-generated`. The fixture must contain such a sibling or the assertion cannot fail.

---

## 5. US4 — trusting that watching stopped when it should

**Set up**: the mock daemon for drops and recovery (`stall`, `close-mid-frame`,
`delay=250,drop=20`), the locally spawned engine for the descriptor count, `FakeFileWatcher` for
the refusal path.

```bash
cargo test -p apex-engine --test watch_lifecycle
cargo test -p apex-engine --test watch_capacity
cargo test -p apex-shell  --test watch_reconnect
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US4.1 | Close a workspace | Every watch released; descriptor count back to the pre-open level |
| US4.2 | Drop the connection | The developer is told changes are no longer reported — a state, not a silence |
| US4.3 | Reconnect | Tree marked stale in full; zero listings until navigation; zero blobs discarded |
| US4.4 | Reconnect with folders expanded and tabs open, including a tab whose folder is collapsed | One `workspace/watch` carrying the whole current set; every member of it present |
| US4.5 | Restart the engine, re-attach | Watching resumes and the client is told events were missed |
| US4.6 | Exhaust watch capacity | `refused[]` returns entries; the workspace still opens and browses; the developer is told which freshness was lost |

US4.4 is why `workspace/watch` takes a list and is idempotent (research.md, *Protocol additions*).
A per-path API makes the client replay a remembered history, and a client that mis-remembers
resumes believing it is being told about changes when it is not.

---

## 6. Every success criterion

All 22. Each row names the command and the **observable** — the thing you look at, not the thing
the code intends.

| SC | Claim | Command | Observable |
|---|---|---|---|
| SC-001 | Open-tab change reflected within 2 s, p99 | `cargo test -p apex-shell --test watch_delivery -- --nocapture` | **Printed** p99 in ms over ≥100 samples, harness delay excluded (§7) |
| SC-001a | Unfocused tab marked, zero focus changes | `npm run e2e` (`file-watch.spec.ts`) | The marked tab's id differs from the focused tab id; focused id identical before and after |
| SC-001b | Open tab in a collapsed folder reported; closed tab not | `cargo test -p apex-shell --test file_event_apply` | Report count 1 for the collapsed-folder tab, 0 after the last tab closes |
| SC-002 | Zero events for excluded paths | `cargo test -p apex-engine --test watch_exclusions` | Event count 0 per exclusion, over every member of the built-in set plus a `.gitignore` entry |
| SC-003 | Watcher and indexer exclusion sets identical | `cargo test -p apex-engine --test watch_exclusions` | **Partial** — the indexer does not exist yet. See *Known gaps* |
| SC-004 | Over the limit: one invalidation, zero events | `cargo test -p apex-engine --test coalescer` | Frame counts by method name at the recording sink: 1 and 0 |
| SC-005 | Interactive budget holds through a 10 000-change burst | `cargo test -p apex-engine --test watch_budget -- --nocapture` | **Printed** p99 for an interactive read during the burst, against §1.4's 250 ms |
| SC-006 | Wholesale invalidation discards zero blobs | `cargo test -p apex-shell --test invalidate_all` | Blob count **and** bytes identical before and after |
| SC-006a | Event on a cached file: zero blobs discarded, zero extra confirmations | `cargo test -p apex-shell --test file_event_apply` | Hash-confirmation count at the fake engine is exactly the one F003's read already makes |
| SC-006b | Unproven content still served while disconnected, marked possibly stale | `cargo test -p apex-shell --test unproven_content` | Bytes returned while the connection source says disconnected, with the possibly-stale presentation |
| SC-007 | 1 000 writes in 1 s bounded by elapsed time / window | `cargo test -p apex-engine --test watch_budget -- --nocapture` | **Printed** event count; bound computed from the window constant the coalescer exports, not a literal in the test |
| SC-008 | Rename: one event naming both paths, content survives | `cargo test -p apex-engine --test coalescer`, `cargo test -p apex-shell --test file_event_apply` | One `renamed` frame carrying both paths; same blob id before and after |
| SC-009 | 100 open/close cycles leak zero watches | `cargo test -p apex-engine --test watch_lifecycle` | Watch-descriptor count read from `/proc/self/fdinfo/<fd>` before and after the loop |
| SC-009a | Watch count proportional to what is open, not to the repository | `cargo test -p apex-engine --test watch_budget -- --nocapture` | **Printed** watch count for a 100 000-file tree with ten folders expanded, against the expected set size |
| SC-009b | Collapse releases; collapse over an open tab releases nothing that tab needs | `cargo test -p apex-engine --test watch_scope` | Count returns to the pre-expansion value; in the tab case the tab's parent is still in the set |
| SC-009c | Exhausted capacity still opens and browses | `cargo test -p apex-engine --test watch_capacity` | `refused[]` non-empty, call returns success, a subsequent `readDirectory` still answers |
| SC-010 | Out-of-root event path refused by the client | `cargo test -p apex-shell --test file_event_containment` | Refusal for each escape shape, including ones the engine itself would have refused |
| SC-011 | Watching unavailable: the developer is told | `cargo test -p apex-shell --test watch_reconnect`, `npm run e2e` | A published state, plus a rendered indication that survives the greyscale check |
| SC-012 | A change made while disconnected shows on the next navigation | `cargo test -p apex-shell --test watch_reconnect` | The listing issued by the simulated navigation returns the new state |
| SC-012a | Reconnection: zero listings until navigation, zero blobs discarded | `cargo test -p apex-shell --test watch_reconnect` | Request count at the recording fake is 0 between reconnect and navigation; blob bytes unchanged |
| SC-012b | Everything expanded and open is watched again | `cargo test -p apex-shell --test watch_reconnect` | The set in the single post-reconnect `workspace/watch` equals the pre-drop set, compared as sets |
| SC-013 | The suite runs with no remote host and no network | `unshare -rn make test` | The full gate passes with no network namespace. Needs unprivileged user namespaces; see *Known gaps* |

---

## 7. The measurements (A-NFR binds all four)

Four criteria are performance or volume claims. For **each** of them A-NFR applies in full, and
the guide states it once so no measurement quietly drops a clause:

- **p99**, not mean and not max.
- Measured at the **interface/transport boundary**, not wall clock end to end.
- **At least 100 samples.**
- **Any delay the harness itself injects is excluded** — the mock daemon's 250 ms in particular.
- **The measured value is printed, not merely compared.** A gate that says only PASS tells nobody
  how much headroom is left, which is what says whether the next feature can be afforded.

```bash
cargo test -p apex-engine --test watch_budget    -- --nocapture
cargo test -p apex-shell  --test watch_delivery  -- --nocapture
cargo test -p apex-shell  --test file_event_budget -- --nocapture
npm run perf:budget        # vitest run tests/perf — picks up new files in that directory
```

Expected shape of the output, with the numbers to be filled by the run:

```
SC-001  event to tab marked            p99 = ___ ms    budget 2000 ms   (n=___)
SC-005  interactive read during burst  p99 = ___ ms    budget  250 ms   (n=___, burst=10000)
SC-007  events for 1000 writes / 1 s   p99 = ___        bound  ___      (window=100 ms)
SC-009a watches held, 100k files / 10  p99 = ___        expected ___
```

**SC-001 needs its measurement point stated, because it is the one that can be measured wrongly
and look right.** The claim is 2 seconds from the write landing on the host to the change being
reflected. That spans two processes. Measure it as **one** interval across the locally spawned
engine — write into the temp tree, stop the clock when the client's projection and tab marking are
updated — because the alternative, summing an engine-side p99 and a client-side p99, is not a p99
of the whole: two independent 99th percentiles composed give a 98th percentile bound, and
reporting that as p99 overstates what was measured. If the single-interval measurement is not
achievable, report the composition **as a p98 bound** and say so.

**SC-007's bound must be derived, not typed.** SC-007 requires the window's value to be read from
the plan rather than assumed. In practice that means the coalescer exports the window as a
constant, the test computes `ceil(elapsed / window)` from it, and widening the window in the
source changes the bound the test uses. A test carrying its own `10` passes after somebody
changes the window to 500 ms, which is the moment the assertion stopped meaning anything.

---

## 8. The negative checks

Each of these is a claim that something does **not** happen. A negative check that cannot fail is
worthless, so each row says what would make it fail — if that condition is not in the fixture, the
check is decoration.

| Must not happen | How you would see it | What makes the check able to fail |
|---|---|---|
| **An event for an excluded path** (SC-002) | Event count per exclusion at the engine's outbound sink, counted before serialisation | The fixture must write into each built-in exclusion **and** a `.gitignore` entry, and must expand `node_modules` first — an implementation that filters at the client, or filters only unexpanded folders, passes otherwise |
| **A blob discarded on wholesale invalidation** (SC-006) | Blob ids and bytes compared before and after, not a row count | The fixture must hold at least two blobs with different content. A count-only assertion passes for a clear-and-refill; identical-content blobs pass for a swap |
| **A listing request on reconnection** (SC-012a) | Request count at the recording fake transport between reconnect and the first simulated navigation | There must be expanded folders at the moment of reconnection. With nothing expanded, an eager re-lister has nothing to list and the check passes for the wrong reason |
| **A focus change when an unfocused tab is marked** (SC-001a) | Focused tab id read before and after, in the running window | There must be **two** open tabs, the changed one unfocused. With one tab the assertion is vacuous; asserting only that the marker appeared passes for a focus-stealing implementation |
| **A report after the last tab closes** (SC-001b) | Report count 0, plus an `unwatch` covering the path | The file's folder must be **collapsed** in the fixture. With it expanded, the watch legitimately stays and the unwatch half of the assertion cannot fire |
| **A fetch for a path never fetched** (FR-020) | Byte counter at the transport fake | The event must name a path absent from `files` and from `file_contents`; an event for a cached path proves nothing here |
| **Content marked valid by an event** (FR-019) | Validity transitions counted, and the hash confirmation on the next read still observed | There must be an assertion that the next read **does** confirm. "The right bytes came back" is true whether or not the event cheated |

---

## 9. Mutation checks

This project's practice: break the thing deliberately, confirm the test fails, revert. Per A-TEST,
a check guarding a property that would otherwise be invisible is verified by breaking the property.
Five worth running here, the first three at minimum.

| Mutation | Test that must fail | What it is really testing |
|---|---|---|
| **Widen the coalescing window** from 100 ms to 3 s | `watch_delivery`'s SC-001 measurement | Whether the suite has a **lower bound** at all. FR-012's assertion is an upper bound on event count, and widening the window makes the count *fall* — so the coalescer unit test passes happily while the developer waits three seconds. If nothing fails, that absence is the finding |
| **Narrow the window** to 1 ms | `watch_budget`'s SC-007 bound | The other side of the same fence: the count explodes past `ceil(elapsed / window)` |
| **Remove the separator boundary** from the subtree rename prefix match — match the prefix alone rather than the exact row plus the prefix with its separator | `rename_subtree` | Whether the fixture contains a sibling sharing the prefix (`src` and `src-generated`). Without one, silently corrupting unrelated rows passes every assertion |
| **Make an event mark content valid** | `file_event_apply`'s FR-019 assertion, and the hash-confirmation count in SC-006a | Whether validity is asserted as *a hash comparison having happened*, or merely as *the right bytes came back*. Only the first can fail here |
| **Drop the client-side containment re-check** | `file_event_containment` | Whether the test injects the escaping path at the **client's** inbound adapter rather than through an engine that already refuses it. Injected further up, the mutation is invisible and Principle VI is untested on the receiving side |

Run each as: apply the mutation, run the named target, confirm it fails with the assertion you
expected rather than a compile error, revert, confirm green.

---

## 10. The gates before merge

```bash
make gate                  # make test, plus npm run e2e and npm run gate:fidelity under xvfb
```

Two design-system obligations are not covered by `lint:ds`, which restricts raw hex, raw pixels and
font families and has no view of either. F004 adds **two new visual states** — a stale tree region
and a changed-tab marker — so both obligations apply to them:

- **A channel other than colour**, asserted in greyscale, following `rail-greyscale.spec.ts` and
  F003's `cache-verification.spec.ts`.
- **Keyboard reachability** for anything the marker makes actionable, following
  `rail-keyboard.spec.ts`.

Per plan.md's Constitution Check, if `mockups/Apex IDE (standalone).html` has no state for either,
that is a deviation needing written designer approval recorded **before** implementation — not an
invented style, and not a fidelity gate quietly updated with `gate:fidelity:update`.

If the browser-driven gates need a display, F003's quickstart records that `xvfb-run` wrapping the
command was not enough and a persistent `Xvfb :77` was; the same applies here.

---

## 11. Validation record

*To be completed when the feature is implemented. Left empty deliberately: a table of results
nobody produced is the failure mode A-TEST names — confidence that has not been earned.*

| Check | Result |
|---|---|
| `make test` | |
| `make gate` | |
| `unshare -rn make test` (SC-013) | |
| The four printed measurements (§7) | |
| The five mutations (§9), each failing then reverted | |

---

## Known gaps

Stated here rather than discovered by a reviewer.

1. **SC-003 cannot be verified as stated.** The criterion compares the watcher's exclusion set
   with the indexer's, as sets. There is no indexer — `workspace/search` is unimplemented and
   belongs to F013. What is runnable today is the structural half: the set is computed once at
   `workspace/register`, stored on the registered workspace, and every consumer reads it from
   there (research.md, *Where the exclusion set lives*). A test can assert there is exactly one
   construction path and that the watcher uses it. That proves *disagreement is impossible by
   construction*; it does not compare two sets, because only one exists. SC-003 becomes fully
   verifiable in F013, and the comparison test belongs in that feature's suite.

2. **The mock daemon cannot yet deliver an unsolicited notification.** Its README restricts
   directives to framing, timing and stream shape, and a guard test fails if any §4.8 method name
   appears in that directory. F004 is the first feature needing a server-initiated frame. Until a
   directive exists that emits a caller-supplied frame, the delivery-under-loss half of SC-001
   has no route through the mock, and the locally spawned engine is the only path.

3. **Real-kernel watch exhaustion (SC-009c) needs privileges.** Lowering
   `/proc/sys/fs/inotify/max_user_watches` requires root, so the exhaustion path is exercised
   against the `FakeFileWatcher` returning refusals. The adapter's own mapping of the kernel's
   `ENOSPC` onto `refused[]` is therefore unproven by default. It belongs in an opt-in suite
   behind an environment variable, alongside the existing `APEX_REAL_SSHD` pattern.

4. **SC-013's network check depends on the host.** `unshare -rn` needs unprivileged user
   namespaces enabled. Where they are not, the criterion falls back to asserting that no test
   opens a socket — weaker, because it proves nothing about a dependency that would.

---

## What "done" looks like

```bash
make test \
  && cargo test -p apex-engine --test watch_budget -- --nocapture \
  && cargo test -p apex-shell  --test watch_delivery -- --nocapture \
  && make gate
```

All green, with the four measured numbers from §7 recorded in the pull request per the
constitution's evidence rule, and the five mutations in §9 each confirmed to fail before being
reverted. A claim that something passes is accompanied by the command and its output, or it is not
a claim.
