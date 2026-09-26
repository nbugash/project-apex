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

---

## 6. Validation record

*(Filled in when the feature is implemented. Numbers, not verdicts.)*
