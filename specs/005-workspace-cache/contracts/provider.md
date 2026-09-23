# Contract: `WorkspaceProvider`

**Feature**: F003 workspace-cache | **Date**: 2026-09-23

§6.1 of `project-apex-predator.md` declares this trait and is normative for its signature. This
document states what an implementation must guarantee, which a signature cannot.

Three implementations exist after this feature: `RemoteWorkspaceProvider` (an adapter over
`RequestTransport`), `CachedWorkspace` (an application-layer use case that wraps another provider),
and `FakeWorkspace` (in-memory, tests only). A fourth, the local provider of §6.4, belongs to F015
— see plan.md's Structure Decision for why it is not built here.

---

## Guarantees every implementation makes

| # | Guarantee | Requirement |
|---|---|---|
| P1 | A caller cannot determine from the interface whether content is local or remote | FR-001 |
| P2 | `read_file` returns bytes. No implementation may transcode, sniff an encoding, or return text | FR-003, §6.1 |
| P3 | A `RelPath` that escapes the workspace root is refused. Every implementation asserts containment for itself | FR-005, §6.2, Principle VI |
| P4 | Unimplemented methods return a typed `Unsupported` error naming the owning feature. They never panic, and never return a plausible-looking empty result | FR-004 |
| P5 | Every method is cancellation-safe: dropping the future leaves no partial write and no leaked in-flight request | §4.5, A-REQ |
| P6 | Errors are typed, never stringly. A caller distinguishes not-found, refused, offline and unsupported without parsing a message | — |

P4 is worth stating as a guarantee rather than an omission. A `write_file` that returned `Ok(())`
without writing would be discovered by F006, weeks later, as a bug in F006.

---

## `RemoteWorkspaceProvider` — additional guarantees

| # | Guarantee | Requirement |
|---|---|---|
| R1 | One provider call issues **at most one** protocol request. It never fans out, retries or prefetches | FR-015, §10.1 |
| R2 | `read_directory` pages internally only when the caller asks for the whole directory; a caller that wants one page gets one request | FR-024 |
| R3 | Reads above the bulk threshold are routed to `BulkTransfer`, not chunked through the channel | FR-025, A-BULK, §4.6 |
| R4 | Requests carry an explicit priority. Tree and file reads are `Interactive`; nothing this feature issues is `Background` | §4.6 |
| R5 | A frame the engine sends is untrusted. A malformed result is an error, never a panic and never a default value | Principle VI |

R1 is the one that makes SC-002 provable: if a provider call could issue two requests, the count
of listings would stop equalling the count of expanded folders and the acceptance test would be
measuring something else.

---

## `CachedWorkspace` — the rules

This is where the feature's behaviour lives. It holds an inner `WorkspaceProvider`, a
`WorkspaceCache`, a `Clock` and a `ConnectionStatusSource`.

### `read_directory`

```
consult the projection for (workspace, path)
  hit  → return it, no request                                  FR-016, SC-002
  miss → one shallow request, persist, return                   FR-015, §10.1
```

Opening a workspace fetches the root listing and nothing else (FR-014, SC-001). A collapsed and
re-expanded folder issues no second request, because the projection still holds it (US1 scenario 3).

Tree entries have no separate validity: a listing is replaced when a request returns a different
one, and invalidated wholesale by §10.4's `workspace/invalidateAll`, which F004 owns.

### `read_file` — connected

```
look up the cache entry
  absent            → fetch, cache if eligible, Presentation::Current
  present           → publish Presentation::Verifying             FR-021b
                      stat with a 2s limit                        FR-021c
                        hash equal     → serve cached, record access, Current
                        hash differs   → fetch, replace, Current   FR-019
                        limit exceeded → serve cached, Unverified   FR-021c
```

Four rules that are easy to get subtly wrong, so they are stated:

1. **Nothing is shown before confirmation.** `Verifying` is published *before* the stat is issued,
   for the entire time it is outstanding, and no bytes reach the caller until it resolves
   (FR-021a, FR-021b, SC-004a).
2. **Git status is not consulted.** There is no code path from `git_status` to this decision. §5.3
   and FR-020 make that a rule, and the absence of a parameter is what enforces it (US2 scenario 5,
   SC-005).
3. **A caching failure is swallowed.** A full disk, a locked database or an oversized file affects
   what is stored, never what is returned (FR-034).
4. **Every hit records an access** in the same transaction that reads it, so retention measures use
   (FR-028, invariant 9 in data-model.md).

### `read_file` — disconnected

```
cached     → serve, Presentation::PossiblyStale                   FR-032
not cached → Presentation::Unavailable, typed error               FR-033
```

No request is attempted. `ConnectionStatusSource` is consulted before the cache, not after a
failure, so an outage costs nothing and produces no timeout (SC-011, SC-012).

### `stat`

Passed through when connected. When disconnected, answered from `files` if the entry is known, and
marked possibly stale on the same basis as content.

---

## `FakeWorkspace` — why it is a contract and not a test helper

The fake is in-memory: a map of paths to bytes, a configurable latency, and switches for "engine
never answers" and "content changed underneath". It exists so that:

- `CachedWorkspace`'s rules are tested with no SQLite file, no process and no network (FR-035).
- The confirmation-limit path (FR-021c, SC-004b) is exercisable at all. A wedged engine is not
  otherwise producible on demand.
- The trait is proven implementable more than once. A port with one implementation is a claim, and
  §6.4's local provider is twelve features away.

It lives beside the tests, not in `src/`, so it cannot be wired into a real composition by accident.

---

## A seam F013 must reconcile deliberately

This feature refuses `search()` on the provider while delivering offline path search through a
separate `SearchPaths` use case. That is consistent with FR-002, which scopes the provider to
listing, metadata and ranged reading — search is not among the three, and FR-001's "one interface"
governs *content*, which search results are not.

But it means that when F013 implements `search()`, there will be two search entry points: one on the
provider for content and online paths, one beside it for offline paths. F013 should decide which
absorbs which rather than discovering the duplication at its second call site. Recorded here because
the choice is cheap now and expensive later.

---

## The contract test

One suite runs against **every** implementation, parameterised over the constructor. It asserts P1
through P6 and, for providers with an engine behind them, R1. Adding F015's local provider means
adding one line to that suite, and the day it fails is the day the abstraction stopped holding.

This is the mechanism by which "the UI never learns which is active" (§6.1) becomes a test rather
than an intention.
