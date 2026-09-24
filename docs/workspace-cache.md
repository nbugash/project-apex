# The workspace cache

What F003 built, where it lives on disk, and the three numbers a developer can actually hit.

For _why_ each decision was taken, see `specs/005-workspace-cache/research.md`. For the
normative values, `project-apex-predator.md` §5 and §6. This file is the operational view.

## What it is

A SQLite projection of a remote workspace, so the interface renders without waiting on the
network. **It is a projection, never an authority** (§5.1). Where it disagrees with the engine,
the engine wins — and the only thing that can make it disagree is a hash comparison.

It does three jobs: renders the file tree instantly, avoids refetching files that have not
changed, and keeps what you have already read available when the connection drops.

## Where it lives

```
<app data dir>/workspace-cache.db        the projection
<app data dir>/workspace-cache.db-wal    write-ahead log
<app data dir>/workspace-cache.db-shm    shared memory index
<app data dir>/session.json              interface state — NOT part of the cache
```

`session.json` is deliberately separate (A-STATE). The two have different lifetimes: the cache
is disposable and expected to be evicted, while window geometry and open documents are a durable
preference. Losing your layout because a cache was cleared would be a defect.

### Deleting it is safe

Everything in the cache is reproducible from the engine. Deleting the three `workspace-cache.db*`
files costs a refetch of whatever you had open and nothing else. **Delete all three** — a
surviving `-wal` is replayed into a fresh database, which is exactly the state you were trying
to be rid of.

The application does this itself when a migration fails or when the file was written by a newer
release, and it tells you. It never refuses to launch over a bad cache: an unusable projection
costs offline access, not your session. If the location cannot hold a database at all — a
read-only filesystem, a full disk — it runs in memory for that session and says so in the log.

## Three numbers you can hit

| Limit              | Value         | What happens when you reach it                                                                                                   | Where it is decided |
| ------------------ | ------------- | -------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| Cache eligibility  | **8 MiB**     | Files above this are read normally but never cached, so they are **not available offline**                                       | A-CACHECAP          |
| Retention          | **14 days**   | Content not _opened_ for 14 days is removed at the next startup. The file stays in the tree and refetches on next open           | §5.5, FR-026        |
| Confirmation limit | **2 seconds** | If the engine does not confirm a file's hash in time, you get the cached copy marked _unverified_ rather than an indefinite wait | A-DEADLINE          |

Two consequences worth knowing before they surprise you:

**A large generated file will not be there on a plane.** Anything over 8 MiB is never cached.
It is reported as unavailable offline rather than shown as an empty document, but it is not
available. That is the trade the cap makes: without it, one clone of a repository with large
binaries fills the disk with content nobody reads twice.

**Eviction only runs at startup.** A session that never restarts never evicts, and disk can grow
for its duration (FR-026b). This is accepted rather than overlooked — the alternative runs
reclaim while somebody is working, and the one thing §1.4 protects is a sidebar that answers in
under a millisecond. A reclaim holding a write lock is precisely what breaks that.

## What the cache does _not_ speed up

**Opening a file while online costs a round trip** (FR-025b). Cached content is not shown until
the engine confirms its hash is current, because a stale byte must never be shown on a working
connection. The cache's online value is the transfer it avoids; its offline value is
availability. It is not an open-latency optimisation, and a later measurement showing an open
taking a round trip is not a regression against a promise.

What it _does_ speed up is browsing: a folder already listed expands from local SQLite, measured
at well under the 1 ms §1.4 budgets.

## Git status is not a cache signal

A file you have just edited and saved is `MODIFIED` in git, and it is still served from the
cache. Cached content is valid exactly when its hash matches the engine's, and **nothing else
invalidates it** (§5.3, FR-019, FR-020). Invalidating on git status would discard cached content
for precisely the files you are working on, forcing a refetch on every save and making them
unreadable offline — the one situation the cache exists for.

## Renames

A rename the client performs keeps its cached content: the projection identifies a file by an
opaque identity rather than by its path, so moving it is an update of three columns.

A rename made elsewhere and noticed when a folder is re-listed **loses** that file's cached copy
and refetches it (FR-022a). A re-listing shows one name gone and another present with nothing
linking them, and recovering the link would mean hashing every entry in the folder to save
refetching one file. The file stays listed throughout; only the cached bytes go.

## Running the checks

```bash
cargo test --workspace          # 417 tests, no network, no remote host
npm run test:unit               # interface logic
cargo test -p apex-shell --test workspace_budget -- --nocapture
```

The last one prints its measurements rather than only comparing them (A-NFR), because a budget
only ever compared against tells nobody how much headroom is left:

```
sidebar expand (cached)    p99 =     80 us   budget   1000 us
sidebar expand (uncached)  p99 =  16128 us   budget 250000 us
compression ratio         36.6 %            budget   50.0 %
```

The end-to-end suite needs a display: `tauri-driver` initialises GTK and cannot start in a
headless environment.
