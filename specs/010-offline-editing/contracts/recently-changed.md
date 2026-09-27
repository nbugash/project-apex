# Contract: `git/recentlyChanged`

Phase 1. The one protocol addition this feature makes. **§4.8 is the source of truth for the
method catalogue; this states the guarantees the table cannot express.**

**Wire spelling is `snake_case`** (A-WIRECASE), as with every method.

---

## `git/recentlyChanged` — request

**Params**: `workspace_id`, `commits?`
**Result**: `{ paths[] }`

### Guarantees

1. **Paths only.** No content, no commit identities, no authors, no timestamps. The method exists
   to feed a prefetch, and a prefetch needs to know *which files*. Widening it into a history API
   is a deliberate act for a later feature, not a side effect of this one.
2. **`commits` defaults to 20 and is capped at 100.** A caller asking for more gets 100. The cap
   is the engine's, as §4.1's frame cap is: a request for ten thousand commits on a monorepo is
   not a request the engine should honour just because it was made.
3. **Paths are workspace-relative and contained**, on exactly the terms `git/getStatus` gives —
   including FR-003a's subdirectory rule, so a workspace on a subtree receives its own paths
   re-rooted and nothing from elsewhere in the repository.
4. **Deduplicated.** A file changed in five of the last twenty commits appears once. The caller is
   building a set.
5. **A workspace that is not a repository, or a host without a usable git, answers successfully
   with an empty list** — not an error, on the same terms as `git/getStatus` (FR-027, FR-028).
   Prefetch on a plain directory caches manifests and nothing else, which is a complete outcome.
6. **Bounded by the frame cap.** Where the result would exceed §4.1's limit it is truncated, and
   truncation is not an error: prefetch is speculative, and a partial answer is a partial
   prefetch, which FR-029a already treats as an ordinary outcome. Unlike `git/getStatus` this is
   **not paged** — a cursor would let a client walk the whole history of a monorepo one page at a
   time, which is the opposite of what a bounded prefetch wants.

### Refusals

| Condition | Code |
|---|---|
| Workspace not registered | `-32001` (§4.4) |
| Workspace root gone | `-32009` |
| `commits` not a positive integer | `-32602` invalid params |

### What is **not** in this contract

- **No commit metadata.** Not the hash, the message, the author or the date.
- **No file content.** Prefetch fetches through `workspace/readFile` like anything else, so the
  §4.6 priority rules and the §4.1 frame cap apply unchanged.
- **No ordering promise.** The caller is building a set; depending on the order would be
  depending on git's output order, which is not a protocol guarantee.
