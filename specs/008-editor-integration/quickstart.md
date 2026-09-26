# Quickstart: Editor Integration

**Feature**: F006 `editor-integration` | **Date**: 2026-09-26

How to prove this feature works, and what to record while proving it. Numbers, not verdicts: a
gate that says only PASS tells nobody how much headroom is left (§1.4, A-NFR).

---

## 1. Prerequisites

```bash
cargo build -p apex-engine --bins --examples
npm run build
```

No instance and no network. The end-to-end suite runs against a real engine started as a child
process — `APEX_LOCAL_ENGINE`, which F010 added — so every scenario below is reproducible on one
machine.

---

## 2. The whole gate

```bash
make gate
```

Which is: `cargo test --workspace`, clippy `-D warnings`, `cargo fmt --check`, the webview unit
suite, ESLint, `svelte-check`, `lint:ds`, the end-to-end suites, the fidelity gate, and
`no-network`.

---

## 3. The measurements (§8 of this guide)

Each prints its value. Record the number beside its bound.

| Criterion | Command | Bound |
|---|---|---|
| **SC-001** requests issued while typing 100 characters | `npm run e2e -- --spec tests/e2e/editor-local-echo.spec.ts` | **0** |
| **SC-002** read requests when opening a cached file | same spec | **0** |
| **SC-005** first visible window of a large file, p99 | `cargo test -p apex-shell --test editor_first_paint -- --nocapture` | < 250 ms |
| **SC-006** chunks transferred before first paint | same test | ≤ 1 |
| **SC-007** largest read response | same test | ≤ 1 MiB |
| **SC-014** writes issued by typing with autosave off | `npm run e2e -- --spec tests/e2e/editor-save.spec.ts` | **0** |

SC-001 and SC-002 count rather than time, because §1.4 budgets the keystroke at "0 ms network".
A duration target of zero is not measurable; the absence of a request is.

---

## 4. The negative checks

Each of these must be able to **fail**. A check whose fixture cannot produce the failure it
watches for is a check that reports success unconditionally.

| Must not happen | The condition that lets it fail |
|---|---|
| A stale write overwrites the host (FR-008) | The file is genuinely changed on disk between read and save, and the test reads the host's bytes back afterwards — not merely the reply |
| A save announces "changed on the host" (FR-024a) | The watcher is actually running and watching the file, so the echo really is emitted; a test with no watcher passes trivially |
| A genuine remote change goes unreported (SC-013) | Something other than the client modifies the file, so the hash really differs |
| A partial buffer is written whole (research.md, *Ranges*) | The file is larger than the threshold and only the first range has been read |
| A path escapes the workspace (Principle VI) | The path really resolves outside the root — a symlink that exists, not a string with `..` in it that canonicalisation would reject anyway |
| A failed write truncates the file (FR-015) | The write really fails after opening, so the rename never happens |
| Autosave writes with no changes (FR-007c) | Autosave is enabled and the buffer is untouched |

### Audit (2026-09-26)

Each row above was checked for whether the condition that *lets it fail* is actually present.
A negative check whose enabling condition is missing passes for any implementation, which is
the failure mode this audit exists to find.

| Check | Condition present? | Evidence |
|---|---|---|
| A stale write overwrites the host | **Yes** | `editor-save.spec.ts` writes real bytes to the file between the read and the save, and reads them back off disk afterwards. The engine test asserts on the file's bytes, never on the reply |
| A save announces "changed on the host" | **No — see below** | The hash comparison is exercised against a real file and a real engine; the *delivery* of the event is synthetic |
| A genuine remote change goes unreported | **Yes** | The file is modified by the test process, so the digest the engine reports genuinely differs from the buffer's base |
| A partial buffer is written whole | **Yes** | `editor-large-file.spec.ts` opens a ~1.4 MB file, past the 512 KiB threshold, and only the first window is read |
| A path escapes the workspace | **Yes** | `engine/tests/write_file.rs` creates a real symlink pointing outside the root. A string containing `..` would be rejected lexically and would prove nothing about canonicalisation |
| A failed write truncates the file | **Yes, after a gap was closed** | The directory's write permission is removed, so the write genuinely fails with the file already present. There was no such test until this audit; see *Outcomes*, mutation 3 |
| Autosave writes with no changes | **Yes, after a gap was closed** | `editor-buffers.test.ts` now saves a buffer nobody has changed and asserts zero writes. The existing test asserted zero writes with autosave **off**, which is a different claim |

**The one that is not satisfied.** No watcher runs in any of these tests, because nothing in the
client forwards `workspace/onFileEvent` to the webview: F004 built `file_event_notification.rs`
and no caller, so the module is declared and never constructed. `editor-echo.spec.ts` dispatches
the event the way the engine's will arrive, which makes the echo rule — the hash comparison that
tells our own write from a colleague's — genuinely testable. What it cannot test is that the
engine emits the event at all. That is F004's gap and is recorded here rather than papered over;
until it is closed, SC-012 is verified against a delivered event rather than an observed one.

---

## 5. The mutation checks

Break the property, confirm the test fails, restore. A test that passes both ways is not a test.

1. **Return the request's hash instead of hashing what was written.** `writeFile`'s round-trip
   test must fail — the client would adopt a base describing bytes that were never on disk.
2. **Compare the base after opening the file for writing rather than before.** The
   conflict test must fail on the file's content, not on the reply.
3. **Write in place instead of renaming.** The truncation test must fail.
4. **Skip the hash comparison on a file event and treat every event as a change.** The
   echo test must fail: saving announces a foreign change.
5. **Suppress every event for an open file instead of comparing.** SC-013 must fail — this is
   the shortcut that makes mutation 4's test pass while breaking the thing it protects.
6. **Let a partially loaded buffer accept edits.** The partial-write test must fail.
7. **Remove the containment check from the write path.** The escape test must fail. If it
   passes, the check being exercised is the client's, not the engine's.

Mutation 5 exists because 4 and 5 are each other's failure modes. A fix for one that breaks the
other looks correct from whichever side you are standing on.

### Outcomes (run 2026-09-26)

Each mutation was applied, the suite run, and the change reverted. Every one failed with the
assertion expected, not with a compile error.

| # | Mutation | Test that failed |
|---|---|---|
| 1 | Return the request's hash instead of hashing what was written | `engine write_file::the_returned_hash_describes_the_disk_and_not_the_request` |
| 2 | Compare the base after writing rather than before | `engine write_file::a_mismatched_base_is_refused_and_the_file_is_untouched` |
| 3 | Write in place instead of renaming | `engine write_file::a_write_that_cannot_be_completed_leaves_the_previous_content_whole` |
| 4 | Treat every file event as a change, skipping the hash comparison | `editor-buffers: says nothing when the change is our own write` |
| 5 | Suppress every event for an open file instead of comparing | `editor-buffers: reports a genuine change against a dirty buffer` **and** `refreshes a clean buffer from the host` |
| 6 | Let a partially loaded buffer accept edits | `editor-buffers: refuses an edit rather than accepting one it would discard` **and** `opens a large file as a window` |
| 7 | Remove the containment check from the write path | `engine write_file::a_symlink_out_of_the_root_is_refused` |

Two of these are worth recording beyond a tick.

**Mutation 3 had no test to fail.** Writing in place instead of renaming left every assertion
green: the mode still survived, no temporary file was left behind, and the content still
arrived. The property nobody had written down is that a write which *cannot be completed* must
leave the previous content whole, and an in-place write cannot offer it — it truncates the
destination before it knows whether the write will succeed. `a_write_that_cannot_be_completed_leaves_the_previous_content_whole`
was added during this audit and kills the mutation. The gap is the point: a test suite that
passes under a mutation is not testing the property it was written for.

**Mutation 7 could not be applied as written.** Removing the containment call does not compile,
because `ResolvedPath` has no constructor outside the resolver (`for_test` is `#[cfg(test)]` and
is not compiled into a binary). The check cannot be skipped without changing a type, which is a
stronger guarantee than a failing test. The mutation was therefore applied one level down — the
canonicalised path's prefix comparison was removed, leaving the lexical check — and the symlink
escape test caught it.

---

## 6. Validation record

Measured 2026-09-26, on the development machine described in §1. Numbers, not verdicts: a gate
that says only PASS tells nobody how much headroom is left, which is what says whether the next
feature's work can be afforded.

| Criterion | Bound | Measured | Headroom |
|---|---|---|---|
| **SC-001** requests issued while typing 100 characters | 0 | **0** | — (an absolute, not a budget) |
| **SC-002** read requests when reopening an open file | 0 | **0** | — |
| **SC-005** first window of a 4 MiB file, p99 over 200 samples | < 250 ms | **80.5 ms** | 3.1x |
| **SC-006** windows to read a 4 MiB file whole | ≤ ideal | **16** (ideal 16) | exact |
| **SC-007** largest single response | ≤ 1 MiB | **256 KiB** | 4.0x |
| **SC-014** writes issued by typing with autosave off | 0 | **0** | — |

SC-005 **includes the test double's own cost**, which is not small: `FakeWorkspace` digests the
whole four megabytes on every call, where a real engine digests once and serves the window from
an open file. The figure is therefore an upper bound on what the system adds, and the budget is
met with the harness's work counted against it. Stated rather than quietly subtracted, because a
measurement whose method is not given is a number.

SC-006's "ideal" is `ceil(4 MiB / 256 KiB)`. The count matters on its own: an implementation
fetching a kilobyte at a time would satisfy every other row here and take four thousand round
trips to read one file.
