# Research: Offline Editing

**Branch**: `feature/F012-offline-editing` | **Date**: 2026-09-27 | **Spec**:
[spec.md](./spec.md)

Every decision below closed a genuine alternative. Those that bind features beyond this one are
marked for Appendix A; the rest bind F012 and live here.

---

## Three-way merge: `diffy`, measured against `git merge-file`

**Decision.** Use the `diffy` crate (0.4, MSRV-compatible with 1.75) for the three-way merge.

**Rationale.** SC-006b requires the client's decision to merge or prompt to match a standard
version-control three-way merge in 100% of cases, and FR-020a names context-aware semantics. That
is a claim about behaviour, so it was measured rather than assumed. Eight cases were run through
both `diffy::merge` and `git merge-file`:

| Case | `git merge-file` | `diffy` | Agree |
|---|---|---|---|
| Changes far apart | clean | clean | yes |
| **Adjacent lines** | **conflict** | **conflict** | **yes** |
| Same line, different content | conflict | conflict | yes |
| One side unchanged | clean | clean | yes |
| Both sides made the identical change | clean | clean | yes |
| Two lines apart | clean | clean | yes |
| Insert vs insert at one point | conflict | conflict | yes |
| Delete vs edit of the same line | conflict | conflict | yes |

**Zero disagreements.** The two rows that decide the design are *adjacent lines* — which must
conflict, and does, which is exactly what choosing git semantics over zero-context line ranges
bought — and *two lines apart*, which must merge cleanly so the feature is not useless.

**Alternatives considered.**

- **Write the merge here.** A three-way merge is a well-understood algorithm and the project
  already parses hunks for F011's gutter. Rejected: the algorithm is not the hard part, agreeing
  with git's *conflict boundary* is, and matching it by construction is worth more than matching
  it by intention. A hand-written merge would need the table above as a test suite anyway, and
  would then be a second implementation of something a crate already does.
- **`git2` (libgit2 bindings).** Would match git by definition. Rejected: a very large native
  dependency, and the merge is the only thing it would be used for. The engine already shells out
  to `git` for F011 rather than linking it.
- **Shell out to `git merge-file`.** Exact by definition and no new crate. Rejected: the merge
  runs on the **client**, which has no guarantee that git is installed — the engine is where git
  lives (§12). Offline is also precisely when spawning processes should be minimised.
- **`similar` / `imara-diff`.** Excellent diff engines; neither ships a three-way merge, so the
  conflict-boundary question would come back.

**What this does not decide.** Whether a conflict is presented as markers in the buffer or as a
side-by-side interface is a design question, below.

---

## Recent-commit prefetch needs a protocol method

**Decision.** Add one method, `git/recentlyChanged`, taking a workspace and a commit count and
returning the paths those commits touched. **This is a protocol addition and belongs in Appendix
A**, because §4.8's catalogue binds every feature. Recorded there as **A-RECENT**, which is the
authority on the shape below; this section keeps the alternatives it closed.

**Rationale.** FR-029 and §11.4 require prefetching files changed in recent commits. Nothing in
§4.8 exposes that: `git/getStatus` reports the *working tree*, which is a different question, and
`workspace/search` searches content. The information lives on the engine's host, where git is.

The method is deliberately narrow — paths only, no content, no commit metadata — so it stays a
prefetch input rather than becoming a history API. F013 or a later history feature can widen it
deliberately rather than inheriting a shape chosen for prefetch.

**Alternatives considered.**

- **Prefetch only the files the developer has opened, plus manifests.** No protocol change, and
  the manifest half of §11.4 still works. Rejected: it drops the half of the requirement that
  makes offline useful for files the developer has *not* opened yet, which is the whole point of
  prefetching rather than caching on demand.
- **Have the client run git.** The client has no repository; the workspace is on the host. Not
  possible in remote mode, which is the mode offline matters in.
- **Reuse `workspace/search`.** It searches content, not history. Encoding "give me recent
  commits" as a search query would be a method pretending to be another method.

---

## Where pending edits live: a new table, not new columns

**Decision.** A new `pending_edits` table keyed by `(workspace_id, relative_path)`, holding the
saved content, **the base content and its hash**, whether the file can be merged as text, and when
it was retained. Schema version 4.

**The base content, not only its hash.** The first version of this decision stored the hash alone,
which is enough to detect that the host has not moved and **not** enough to merge — a three-way
merge needs the base text, and the only other copy is in `file_contents`, which this very section
points out is evictable, and which any refetch overwrites. Analyze run 2 found it by asking where
the base text would come from after an eviction; every artifact agreed with every other and they
were wrong together.

**Rationale.** Three reasons, in order of weight.

A pending edit **outlives the cache entry it came from**. `file_contents` is subject to eviction
(F005); a pending edit must not be. Putting them in one row makes eviction a question of "which
columns may I clear", which is exactly the kind of rule that gets it wrong once.

A pending edit **exists for paths that have no `file_contents` row at all** — a file created
offline has no cached content and no base. F011 learned this shape the expensive way: git status
was keyed by the tree's file identity and could not describe an untracked file in a folder the
tree had never listed. Keying by path avoids repeating that.

The two have **different lifetimes and different meanings**. `file_contents` answers "what did
the host last give me"; a pending edit answers "what has the developer written that the host has
not seen". Merging them into one row would make `sha256_hash` mean two things depending on
another column.

**Alternatives considered.**

- **Columns on `file_contents`.** Fewer tables, one join fewer. Rejected for the three reasons
  above; the join is not on any measured path.
- **A file per pending edit on disk.** Survives database corruption and is trivially inspectable.
  Rejected: two stores to keep consistent, and the atomicity FR-028 needs across several files is
  exactly what a transaction gives and a directory of files does not.

---

## Reconciliation is driven by the connection state, not by a timer

**Decision.** Reconciliation runs once per transition into `Connected`, driven by F001's
connection state that the client already observes — the same source FR-001 requires the offline
indicator to use.

**Rationale.** Principle II. There is already one authority on whether the client is connected,
and F011 demonstrated the cost of a second opinion: the git watch and the workspace watch
answered different questions about the same repository until A-GITNUDGE reconciled them. A timer
or a probe would be a second detector of a state that is already published.

**Alternatives considered.**

- **Reconcile on the first successful request after an outage.** No new trigger at all. Rejected:
  it makes reconciliation a side effect of whatever the developer happened to do next, so the
  same outage reconciles at different times depending on unrelated activity.
- **Reconcile on demand, from a button.** Predictable and auditable. Rejected: FR-019 requires
  the clean case to complete with zero interactions, and a button is an interaction.

---

## Unmergeable files are decided by what the client holds, not by sniffing

**Decision.** A file is unmergeable when the client does not hold it as text — it was never
decoded as UTF-8, or it exceeds the editor's existing 64 MiB text limit. No content sniffing.

**Rationale.** The client already makes this decision when it opens a file: F006's `decode_chunk`
either produced text or did not, and `MAX_TEXT_FILE` already bounds it. Reusing that answer means
the set of files that are unmergeable is exactly the set the editor already treats differently,
so a developer meets no new category. A separate binary-detection heuristic would be a second
opinion about the same file, and the two would disagree on some file eventually.

**Alternatives considered.**

- **Sniff for NUL bytes, as git does.** Familiar and cheap. Rejected: it can disagree with the
  decoder that already ran, and the decoder's answer is the one the editor acted on.
- **Ask the engine.** It knows the file too. Rejected: offline is when this matters, and the
  engine is unreachable.

---

## Prefetch stops rather than evicts

**Decision.** Prefetch checks the cache budget before each fetch and stops when continuing would
require eviction. Stopping is reported as a normal outcome.

**Rationale.** FR-029a, and the reasoning is the reviewer's from clarify: speculative content
must not displace content the developer actually opened. Least-recently-used eviction is
precisely wrong here, because the files a developer opened days ago are the ones they will want
when the connection drops on the train home.

**Alternatives considered.** A byte budget with normal eviction, and unbounded prefetch, were
both considered and rejected during clarify; the rationale is recorded in spec.md's
`## Clarifications` rather than repeated here.

---

## Decisions that bind beyond F012

These go to Appendix A of the system specification before implementation begins (Principle III),
because each closes an alternative for features other than this one.

| Decision | Why it binds beyond F012 |
|---|---|
| `git/recentlyChanged` joins §4.8, as **A-RECENT** | The protocol catalogue is shared; any later history or prefetch feature inherits this shape |
| Pending edits are a separate table keyed by path | F013's search and any later feature reading the cache must know a path can have work with no cached content |
| Reconciliation is triggered by the published connection state | Any later feature reacting to reconnection uses the same trigger rather than adding a second |
