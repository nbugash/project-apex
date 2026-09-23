# Quickstart: Validating the Workspace Cache

**Branch**: `feature/F003-workspace-cache` | **Date**: 2026-09-23 | **Plan**: [plan.md](./plan.md)

How to prove this feature works, in the order a reviewer should run it. Every scenario below maps
to a user story or a success criterion, and **none of them needs a remote host or a network**
(FR-035, SC-014).

For what each interface promises, see [contracts/](./contracts/). For the schema, see
[data-model.md](./data-model.md). This document does not restate either.

---

## Prerequisites

```bash
# Rust toolchain, already required by F000-F002
cargo --version            # 1.75 or newer

# Node, for the end-to-end and design gates
node --version             # 20 or newer
npm ci

# No sshd, no EC2 instance, no network. That is the point.
```

The `zstd` and `rusqlite` crates build bundled C. On a clean machine that needs a C compiler:

```bash
cc --version               # any of gcc/clang
```

If `cc` is missing the build fails at dependency compilation with a message naming the crate,
which is a clearer failure than a runtime one and is the reason `bundled` was chosen
(research.md, "The SQLite driver").

---

## 1. The whole suite

```bash
cargo test --workspace
npm run test:unit
```

Expected: all green, no network access attempted. `cargo test --workspace` covers the engine, the
client core and the shared protocol crate.

**Read the summary line, not the exit code.** F001 found a gate reporting `failed=no` while six
tests failed, because `test result: FAILED.` puts the word in the third field and the check looked
at the second. If a number other than zero appears after `failed:`, the run failed regardless of
what anything else says.

---

## 2. Lazy tree loading — US1, SC-001, SC-002

```bash
cargo test -p apex-shell --test workspace_tree
```

What it proves:

| Scenario | Assertion |
|---|---|
| US1.1 | Opening a workspace issues **exactly one** `workspace/readDirectory` |
| US1.2 | An unexpanded folder has issued none |
| US1.3 | Collapse and re-expand issues none |
| US1.4 | Expanding ten folders of a hundred-thousand-file tree issues ten |

The count comes from a recording fake transport, not from a log. A test that greps a log for
request lines passes when the logging changes shape; a test that counts calls does not.

To see the shape of the tree used:

```bash
cargo test -p apex-shell --test workspace_tree -- --nocapture | head -40
```

---

## 3. Cache validity — US2, SC-003, SC-004, SC-005

```bash
cargo test -p apex-shell --test cache_validity
```

| Scenario | Assertion |
|---|---|
| US2.1 | Matching hash: zero content bytes transferred |
| US2.2 | Changed hash: fresh content fetched, cache replaced |
| US2.5 | File marked `MODIFIED` in git, content unchanged: **served from cache** |
| US2.6 | File larger than one message: arrives in ranges, first range returned before the last |

US2.5 is the one worth watching run. It is the scenario that fails if anyone ever wires git status
into validity, and §5.3 says that mistake was already made once in this project's history.

---

## 4. The verification window — SC-004a, SC-004b

```bash
cargo test -p apex-shell --test cache_verification
```

| Assertion | Requirement |
|---|---|
| `Verifying` is published before the stat is issued and stays published until it resolves | FR-021b |
| No bytes reach the caller before confirmation, across every ordering the fake can produce | FR-021a, SC-004a |
| A fake engine that never answers ends the wait at the limit and yields `Unverified` | FR-021c, SC-004b |

The wedged-engine case uses the fake's "never answer" switch and a fake clock, so it completes in
microseconds rather than in two seconds of real time. A test that actually sleeps for the timeout
is a test nobody runs on every commit.

---

## 5. Path containment — SC-006

```bash
cargo test -p apex-engine --test path_containment
```

This one runs against a **real filesystem tree**, in a temp directory, because the symlink case
cannot be proven against a fake. It asserts:

- `../../etc/passwd` is refused with `-32002`
- A symlink inside the workspace pointing outside it is refused
- An escape to a path that exists and one that does not produce **the same** error (FR-007)
- A client that sends an unchecked path is still refused — the engine's check does not depend on
  the client having one (FR-008)

The last is the constitution's Principle VI made executable, and F002's plan recorded that this
feature is where it lands.

---

## 6. Workspace identity — US3, SC-007

```bash
cargo test -p apex-shell --test workspace_registry
```

Two workspaces with the same display name, both populated, each reading back its own content
(SC-007). Re-opening attaches rather than duplicating (US3.2). Deleting removes content and tree
(US3.3).

---

## 7. Retention and maintenance — US4, SC-008, SC-008a, SC-013, SC-013a, SC-013b

```bash
cargo test -p apex-shell --test cache_maintenance
```

Runs against a **real database file** in a temp directory, because the invariants being checked are
about SQLite's behaviour and a fake would be asserting our own beliefs about it.

| Assertion | Requirement |
|---|---|
| Content aged past fourteen days is removed; every `files` row survives | FR-026, FR-027, SC-008 |
| Zero evictions occur while a workspace is open | FR-026a, SC-008a |
| An evicted file re-opens with no error surfaced | FR-029 |
| A v0 database migrates to v1 with content preserved | FR-018, SC-013 |
| A migration killed mid-step leaves the **old** version, intact and readable | FR-018c, M2 |
| A migration that fails deterministically discards and rebuilds, and says so | FR-018b, SC-013b |
| Progress is published at least once per second — asserted on **count and spacing** | FR-018a, SC-013a |

Ageing uses a fake clock. The interrupted-migration case opens the file, begins a step and drops
the connection without committing, which is what a kill looks like to SQLite.

The last row is the one that regressed twice during specification: an assertion that *a* progress
message was sent passes for an upgrade that then hangs silently. The test counts reports and
measures the gaps between them.

---

## 8. Offline — US5, SC-011, SC-012

```bash
cargo test -p apex-shell --test cache_offline
```

The connection source is a fake set to disconnected. Assertions: path search returns results with
**zero requests attempted**; a cached file is served marked possibly stale; an uncached file
produces a stated reason rather than an empty document.

"Zero requests attempted" is counted at the transport fake. Asserting only that the search
*succeeded* would pass for an implementation that tried the network, timed out, and fell back.

---

## 9. The performance gates — SC-004c, SC-010

```bash
npm run perf:budget
```

Prints measured values, not only a verdict (A-NFR):

```
sidebar expand (cached)    p99 = ___ ms   budget 1 ms
sidebar expand (uncached)  p99 = ___ ms   budget 250 ms
compression ratio          ___ %          budget 50 %
```

p99 over at least 100 samples, measured at the interface/transport boundary with harness-injected
delay excluded. A gate that only says PASS tells nobody how much headroom is left, which is what
says whether the next feature's work can be afforded.

The compression ratio is measured over a real source tree — this repository — rather than over
generated text, which compresses far better than code and would make the budget meaningless.

---

## 10. End to end — the acceptance scenarios

```bash
npm run e2e
```

Linux only, per A-E2E. Three new specs:

| Spec | Covers |
|---|---|
| `tests/e2e/workspace-tree.spec.ts` | US1 through the real interface |
| `tests/e2e/cache-verification.spec.ts` | FR-021b's indicator is visible while a confirmation is outstanding |
| `tests/e2e/cache-maintenance.spec.ts` | FR-018a's migration state is visible during an upgrade |

Screenshots land in `reports/screenshots/${OS}/F003/` — the **feature map identity**, not the spec
directory number `005`. The segment is derived from the git branch by `tests/e2e/wdio.conf.ts`.

---

## 11. The gates that must pass before merge

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
npm run lint:ds          # design token adherence, Principle I
npm run gate:fidelity    # mockup fidelity
```

Clippy is not a formality here. In F002 it caught a `std::thread::sleep` inside an async function —
a real defect that would have stalled a runtime worker, found by a lint rather than by a test. This
feature has a blocking boundary of its own at the SQLite adapter, which is exactly the shape that
mistake takes.

---

## 12. Opt-in: against a real `sshd`

```bash
APEX_REAL_SSHD=1 cargo test -p apex-shell --test workspace_real_sshd -- --ignored
```

Proves the one thing no local spawn can: that a bulk read attaches to the existing control master
and does **not** become one. F002 learned this the expensive way — a deployer that became its own
`ControlMaster` with `ControlPersist` backgrounded itself holding the inherited stdout pipe and
hung forever waiting for an EOF that could not arrive. The bulk fetcher makes the same invocation
and carries the same `ControlMaster=no`.

Excluded from the default suite because it needs a real `sshd`, which SC-014 forbids requiring.

---

## What "done" looks like

```bash
cargo test --workspace \
  && npm run test:unit \
  && npm run perf:budget \
  && npm run e2e \
  && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo fmt --all -- --check \
  && npm run lint:ds
```

All green, with the performance gate's three measured numbers recorded in the pull request per the
constitution's evidence rule. A claim that something passes is accompanied by the command and its
output, or it is not a claim.
