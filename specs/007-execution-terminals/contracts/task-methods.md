# Contract: Task Methods

**Feature**: F010 execution-terminals | **Date**: 2026-09-24

The seven methods a client calls to run, reach and end a task: four requests —
`execution/runTask`, `execution/attach`, `execution/list`, `execution/terminate` — two
notifications — `execution/writeStdin`, `execution/resizePty` — and one workspace method whose
whole obligation is executional, `workspace/close`. `project-apex-predator.md` §4.8 is the source
of truth for the catalogue; this document states the guarantees its table has no room for.

**The catalogue was amended on 2026-09-24 and this contract now matches it.** Three rows were
added — `execution/attach`, `execution/list` and `workspace/close`; `runTask` gained `cols?` and
`rows?`; `attach`'s result gained `exitCode?` and `signal?`; `onExit` became `exitCode?` plus
`signal?`; §4.4 narrowed `-32006` to "Task not found" and added `-32010` and `-32011`; plan.md's
*Fixed Quantities* table fixed fourteen values four requirements said it owed. Every row, code and
quantity cited below was read from those files, not carried from a summary of them. What this
contract previously listed as *Amendments still required* has been applied in full; what is left
is in *What was open here, and is not any more*, whose two entries have both since been closed.

Rationale for the shapes below is in research.md, *Attaching to a task that is already running*,
*What the `pty` parameter means, and what it costs* and *Backpressure comes for free*. It is not
restated here. The decisions are **A-TASKLIFE**, **A-TASKLIMIT** and **A-TASKSTREAM**.

---

## What is shared by every method here

**A task identity is chosen by the client** (FR-001, §4.8) and is a bare string. **It is unique
across the engine, not within a workspace** — §4.8 now states this, and gives the reason this
contract derived: six of the nine execution rows address a bare `taskId`, so a per-workspace
identity would leave them unable to resolve a task at all. `workspaceId` on `runTask` and `attach`
records which workspace **owns** the task, not which namespace its name lives in. Two workspaces
that both choose `build` have named one task, and the second `runTask` is refused (`-32010`)
rather than quietly starting a second process.

**`workspaceId` on `runTask` and `attach` says where the task runs, and is validated as F003
validates it.** Unknown or unregistered is `-32001`; registered with a vanished root is `-32009`
(§4.4). **`attach` with a `taskId` that is live under a different workspace is also `-32001`**,
and §4.8 states both the rule and the reason: because a `taskId` is engine-unique the engine could
resolve the task from the id alone and treat the mismatched workspace as noise, but a client that
believes a task belongs to a workspace it does not is a client whose state has diverged, and
silently servicing the request would leave it diverged. Principle VI puts the check on both sides.

**So "not yours" and "not there" are deliberately distinguishable**, which reverses the reasoning
this contract previously gave for merging them. That reasoning was non-enumerability — a caller
able to tell the two apart could enumerate another workspace's tasks from refusals alone — and two
amendments retired it. `execution/list` makes enumeration a **method** rather than a side channel,
and §15.5 with A-EC2 make the instance single-tenant, so there is no second party from whom
another workspace's tasks are hidden. What is left is a client-correctness concern, and for that a
divergence reported beats a divergence serviced. `-32006` accordingly means one thing only: no
live identity of that name, anywhere in the engine.

**`cwd` is untrusted input and §4.7 is normative for it** (FR-003, Principle VI). §4.7's own
sentence names `relativePath`; `runTask`'s parameter is called `cwd`, and the check is identical
and mandatory regardless — the engine canonicalises, asserts containment, and refuses with
`-32002`. That an escape fails the **whole call** and starts no process is the same rule F004's
`workspace/watch` applies for the same reason: degrading a boundary check to a partial outcome
makes it advisory, which Principle VI forbids.

**Wire spelling is snake_case** (§4.8). `workspaceId` is `workspace_id`, `taskId` is `task_id`,
`exitCode` is `exit_code`; `command`, `cwd`, `env`, `pty`, `data`, `cols`, `rows`, `signal`,
`pid`, `running`, `retained` and `tasks` are single words already.

**Adding a method does not increment `protocolVersion`** (§4.8). An engine without
`execution/attach`, `execution/list` or `workspace/close` answers `-32601`, which is exactly the
signal a client needs to know it must redeploy — F004's reasoning for `workspace/watch`,
unchanged, and it now covers three rows rather than one.

---

## `execution/runTask`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `taskId` | string, client-chosen | yes |
| param | `command` | array of string — program, then arguments | yes |
| param | `cwd` | string, workspace-relative; `"."` is the root | no — absent is the root |
| param | `env` | object, string to string | no — absent inherits the engine's, present merges over it |
| param | `pty` | boolean | yes |
| param | `cols` | integer, > 0 — meaningful only when `pty` is true | no |
| param | `rows` | integer, > 0 — meaningful only when `pty` is true | no |
| result | `pid` | integer, host process id | yes |

### Guarantees

1. **`command` is an argv vector, never a shell line** (§4.8). The engine interposes no `sh -c`.
   The catalogue now states this and gives the reason: §7.3 scopes the engine to process
   execution and not a shell, and a single string would make quoting the engine's problem for
   input it is specifically required not to interpret. A client that wants a shell names the shell
   as `command[0]` — which is what the spec's Assumptions mean by "what it starts is a shell — a
   command like any other".
2. **Starting is not idempotent, and that is the requirement.** A `taskId` whose identity is
   already live is refused with **`-32010`**, no second process is started, and the refusal is
   distinguishable from every other failure (FR-031c, SC-022, US5 scenario 4). Making `runTask`
   idempotent was considered and rejected in research.md, *Attaching to a task that is already
   running*: it makes the two outcomes indistinguishable at the call site, which is the failure
   FR-031c names. §4.4 says the correct client response to `-32010` is to **attach, not retry**.
3. **A command that cannot be started fails the request with `-32011`, and never appears as a
   task that ran.** FR-004 and SC-015 admit no other outcome: reported as a start failure in 100%
   of exercised cases and as a task exiting in zero. A spawn failure MUST NOT be answered with a
   `pid` and a following `onExit`. `-32011` and not `-32003`: §4.4 reserves `-32003` for a path
   inside a workspace, and a program name resolved against `PATH` is not one.
4. **The environment is the engine's, with `env` applied over it** (spec Assumptions). Starting
   from empty would leave no `PATH`, and every task would fail for one reason. `env` overrides
   per key; it does not replace the set.
5. **`env` is never logged, never included in an error `message` or `data`, and never reaches a
   crash report** (FR-005a, SC-025) — **including when the task fails to start**, which is the
   path on which a diagnostic would most naturally quote the request, and which is now the
   `-32011` path specifically. The obligation is structural at the port: see runner-port.md,
   guarantee T10. FR-005a also disables core dumps for the task process, because a dump is a
   crash report carrying the whole environment; that is `RLIMIT_CORE` = 0 in runner-port.md,
   *Resource limits*, and it is a requirement rather than a tuning choice.
6. **`pty` chooses one of two shapes and the choice is exclusive** (A-TASKSTREAM, FR-008,
   FR-008a). `true` gives a pseudo-terminal, `isatty` is true, and output arrives merged on
   `onStdout` with `onStderr` carrying nothing. `false` gives separate pipes, the two streams are
   distinguishable, and `isatty` is false. The consequences for delivery are task-events.md,
   guarantee 4.
7. **`cols` and `rows` size the terminal at creation, and are meaningful only when `pty` is
   true** (§4.8). A process reads its terminal width at startup, before any client has had an
   opportunity to resize it; without these the process reads the size the engine created the
   pseudo-terminal with. They are ignored for `pty: false`, where there is no terminal to
   size — the same tolerance `resizePty` extends for the same reason (guarantee 3 there).

   **When they are absent the terminal is created 80 x 24, and §4.8 says so in as many words**:
   "Omitted, they default to **80 by 24** — the conventional terminal size, and specifically not
   the kernel's own default of zero by zero, which is both a size no display has and the one value
   `resizePty` refuses." plan.md's *Fixed Quantities* carries the row, and the engine applies it in
   the use case rather than in the runner, so the quantity is one FR-006b's plan fixed and not one
   the adapter invented (runner-port.md, `Shape::Pty`). A client that knows its panel's dimensions
   SHOULD still pass them, because 80 x 24 is a size somebody chose and not the client's; a client
   that omits them and later learns its width sends `execution/resizePty`, which is the ordinary
   path and no longer a repair. See *What was open here, and is not any more*, item 1.
8. **The task runs in its own process group** (FR-006a), which is what makes terminating it
   terminate what it spawned (FR-018, SC-027) rather than only the command named.
9. **The task runs under per-process resource limits** (FR-006, FR-006b, A-TASKLIMIT): 16 GiB of
   address space, soft **and** hard, and no core dumps. CPU time and process count are
   deliberately not limited. The values and the reasoning are plan.md's *Fixed Quantities*; what
   the limits do **not** bound is runner-port.md, *Resource limits*, and is recorded rather than
   closed.
10. **The task runs as the developer's own user, with no escalation** (FR-005, A-SEC, A-EC2).
11. **A returned `pid` means the process exists.** It is the host process id, the thing §15.2
    already says the engine tracks. It is the process **group leader** by guarantee 8.
12. **The identity becomes live when this call succeeds and stays live until its exit has been
    delivered** (FR-023). Reuse before then is guarantee 2's refusal; reuse after it is a fresh
    task.
13. **A connection drop does not end the task** (A-TASKLIFE, FR-031, SC-018). This is a deliberate
    divergence from F004, where dropping the connection empties the watch set
    (watch-methods.md, `workspace/watch` guarantee 12), and from §7.3, which stops language
    servers on disconnect. A-TASKLIFE gives the reason: the developer asked for the build and did
    not ask for the language server. **`workspace/close` is the event that does end it** (FR-024),
    and §4.8 states the distinction: a drop is an accident, a close is an intention.

### Errors

Each fails the whole call. No process is started and no identity becomes live.

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` (§4.4). The client registers and retries |
| `-32009` | Registered, and the root no longer exists (§4.4). The client tells the developer; it does **not** re-register |
| `-32002` | `cwd` escapes the workspace root, lexically or after a symlink resolves (§4.7, FR-003) |
| `-32003` | `cwd` is inside the root and is not there, or is not a directory |
| `-32010` | The identity is already live (FR-031c, SC-022). The client **attaches**; it does not retry |
| `-32011` | The command could not be started — not found, not executable, or `cwd` unusable (FR-004, SC-015). Carries a `data` object with the underlying reason (§4.4) and **not** the environment (FR-005a) |
| `-32602` | `command` absent, not an array, or empty; `env` not an object of strings; `pty` not a boolean; `taskId` absent or empty; `cols` or `rows` present and not a positive integer |

`-32006` is **not** returned by this method. It means an identity that is not live, which for
`runTask` is the success precondition rather than a failure.

---

## `execution/attach`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `taskId` | string | yes |
| result | `pid` | integer | yes |
| result | `running` | boolean | yes |
| result | `retained` | integer, **byte count** | yes |
| result | `exitCode` | integer | **exactly when** the task ended with a code |
| result | `signal` | string, signal name | **exactly when** the task was killed by a signal |

### Guarantees

1. **Attaching is a different call from starting, and that is the entire point** (FR-031c). A
   client that has reconnected and is not yet certain what survived calls this; a client that
   means to start calls `runTask`. Neither can silently become the other. A client that has lost
   the identities altogether calls `execution/list` first.
2. **`retained` is a byte count, not the bytes** (§4.8). The output produced while no client was
   attached is replayed afterwards as ordinary `onStdout`/`onStderr` notifications, chunked
   exactly as live output is, and `retained` says how much is owed. §4.8 now states both the type
   and the replay, with the three reasons this contract derived: the retention bound — 4 MiB per
   task (plan.md) — is larger than §4.1's 1 MiB frame cap, so the bytes cannot fit in a result at
   all; chunking is defined for notifications and not for responses; and an exit carried in this
   result would arrive **before** the output it followed, which FR-022 forbids. A client that
   needs the bytes enumerated does not need a different shape; it needs to read its notifications.
3. **Delivery order after a successful attach is fixed** (FR-031b, FR-022, SC-019, SC-020):
   **the response first**, then everything retained in the order produced, then anything produced
   since, then — if the task has ended — `execution/onExit`. Nothing produced after the attach
   overtakes anything retained. §4.8 states the same ordering from the other side, and now states
   which method each replayed chunk uses: "the retained bytes are replayed after the response as
   ordinary `onStdout` and `onStderr` notifications — **each chunk on the notification its own
   stream would have used when live** — in order, so one ordering rule covers live and replayed
   output alike." That per-stream rule is what SC-028 measures for a `pty: false` task: replaying
   both streams on `onStdout` would merge the separation that task asked for by not requesting a
   terminal. For a `pty: true` task it is the same sentence with nothing to choose, because
   `onStderr` never carries a byte (A-TASKSTREAM).
4. **A task that has already exited attaches successfully.** `running` is `false`, `retained` is
   what is still owed, and the exit follows as `onExit` once that output has been delivered
   (FR-031b, US5 scenario 3, SC-020). Attaching to an exited task is **not** an error, and §4.4's
   wording no longer says otherwise: `-32006` is "Task not found", full stop.
5. **`exitCode` and `signal` in the result say how it ended, and exactly one of them is present**
   (§4.8, FR-021, SC-020). They are present only when `running` is `false`. `running: false` on
   its own says the task is over and not how it ended, which is what SC-020 asks the reattaching
   client to learn — and learning it from the response means a client can render "failed with
   101" before the replayed output has finished arriving. The `onExit` notification still follows
   (guarantee 3); this result is not a substitute for it, because a client attached throughout
   never sees this response at all.
6. **An identity that was never started, or whose exit has already been delivered and identity
   released (FR-023), is `-32006`.** These two are one condition on purpose: after release the
   identity carries no history, so "never started" and "finished and collected" are the same fact
   about the engine's state, and a code that told them apart would have to describe a state the
   engine does not keep. A `taskId` that **is** live but under another workspace is not this
   condition and is not this code — it is `-32001` (*What is shared*, above).
7. **Attaching twice is not an error and delivers nothing twice.** A client that re-attaches
   having already received the retained output receives `retained: 0`. What the engine tracks is
   **what is still owed, as the FIFO drain of the one retention buffer** — not per-attachment
   bookkeeping. There is **one attachment per task** (architecture.md; `Task::attached` is a
   boolean, data-model.md), so "what this attachment has been sent" and "what has not been drained"
   are the same fact, and a reader must not take the phrasing as requiring state the engine does
   not keep. Many viewers on one task remains a scope question spec.md leaves open, and it is the
   answer that would make the two diverge.
8. **`pid` is returned for a task that has exited.** It is the id the process had. §15.2's PID
   mapping is what makes reattachment across a **disconnection** possible at all — and only a
   disconnection: §15.2 was narrowed to say that the map is engine memory and dies with the
   process, so a disconnection is survivable and a crash is not, and A-TASKEXEC adds that tasks do
   not survive a re-execution either. A client that cannot see the pid of a task it is being told
   about cannot reconcile with the host.
9. **Attaching neither starts, stops, resizes nor writes.** It has no side effect on the process.
   In particular it does not resize the pseudo-terminal to the reattaching panel's dimensions;
   that is a `resizePty` the client sends itself, and a client that forgets to leaves the process
   laying out to the old width.

### Errors

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` — **or** a `workspaceId` that does not own the named task (§4.8). Both are the client's state having diverged from the engine's |
| `-32006` | No live identity `taskId` — never started, or already released (guarantee 6) |
| `-32601` | This engine predates `execution/attach`. The client redeploys (§3.8, A-BOOT) |

`attach` does **not** answer `-32009`, for the reason `execution/list` and `workspace/close` do not:
neither finding a task, nor watching one, nor ending one may be blocked by the disappearance of a
directory. The task is already running and its working directory was resolved when it spawned, so
attaching reads no root. Answering `-32009` here would leave a developer whose workspace root was
deleted able to enumerate a live task and kill it but not watch it, which defeats FR-031b's
unqualified "MUST be able to reattach" for an edge case spec.md lists by name. `runTask` still
answers `-32009`, because starting a task does resolve a root.
| `-32602` | `taskId` absent or not a string |

---

## `execution/list`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | no — omitted lists every task the engine holds |
| result | `tasks` | array of task summaries | yes — possibly empty |

Each element:

| Field | Type | Required |
|---|---|---|
| `taskId` | string | yes |
| `workspaceId` | string, the owning workspace | yes |
| `command` | array of string, as given to `runTask` | yes |
| `pty` | boolean | yes |
| `pid` | integer | yes |
| `running` | boolean | yes |
| `exitCode` | integer | **exactly when** it ended with a code |
| `signal` | string, signal name | **exactly when** it was killed by a signal |

**These are §4.8's fields, not this contract's.** The catalogue names the element as well as the
row, under the same exactly-one rule `onExit` carries, and it states one deliberate omission:
**no `env`**, because FR-005a keeps a task's environment out of anything that can be read back and
a listing is exactly that. Nothing is added here — in particular **not `retained`**: a caller that
wants the byte count attaches, which returns it, and an element that carried it would invite a
client to render "1.2 MB missed" for a task it has not attached to and is owed nothing of.

### Guarantees

1. **It exists for the client that lost its identities, and for nothing else** (SC-023, spec
   Assumptions, §4.8). `attach` takes an identity the caller must already know; a fresh install, a
   cleared profile or a crash before the client's store was written leaves a client with none,
   and under A-TASKLIFE those tasks keep running. Without enumeration they stay unreachable until
   A-EC2's idle stop ends the instance — precisely the abandoned process FR-025 forbids, arrived
   at by a client doing nothing wrong. The durable client store is **A-STATE's** (F000
   `app-shell`), whose payload does not include task identities, which is why this route is
   needed rather than merely convenient.
2. **It is a pure read.** Listing does not attach, does not deliver a byte, does not resize, does
   not release an identity and does not start anything. Calling it a hundred times changes
   nothing, and a client may call it before deciding whether to attach or to terminate.
3. **It is a snapshot of what the engine knows, not a probe of the kernel.** `running: true`
   means the engine has not yet observed the process end; a task that exited a moment ago is
   still listed as running until its reader thread reaches `Ended` and reaps. A client MUST NOT
   treat a listing as proof that a process is alive now — the same caution `runTask`'s `pid`
   carries, for the same reason.
4. **It lists live identities, and only those** (FR-023). A task whose exit has been delivered
   and whose identity has been released is absent — there is nothing left to list. A task that
   has ended and whose exit has **not** been delivered is present, with `running: false` and the
   `exitCode` or `signal` it ended with, because its identity is still live and still owed to
   somebody.
5. **`workspaceId` omitted lists every task the engine holds, across every workspace** (§4.8).
   This is deliberate and is the case the method exists for: a client that lost its store has also
   lost which workspace a task belonged to. Under §15.5 and A-EC2 the instance is single-tenant,
   so there is no second party from whom those tasks are hidden.
6. **`workspaceId` present filters to that workspace**, and an unknown one is `-32001` rather
   than an empty list. An empty `tasks` array is a positive assertion that a registered workspace
   holds no tasks — the same distinction `session/onRestart`'s `unpreserved` draws between an
   empty list and an absence of information.
7. **No element carries `env`** (FR-005a, SC-025, and §4.8 says so in the row's own prose). The
   environment appears in no result, no log line and no crash report, and a listing — a thing
   built to be read back — is the most tempting place to put it.
8. **Every element carries `command`, and FR-005a deliberately does not protect it.** The spec
   records a credential passed as an argument as a known, accepted boundary rather than an
   oversight. Listing it widens that boundary from "the client that started the task" to "any
   client that can list" — which under single tenancy is the same developer, but is a widening
   and is stated rather than left to be discovered.
9. **The result is ordered by `taskId`, byte-wise on the UTF-8 encoding, and no client behaviour
   may depend on that.** The order is the `TaskSet`'s `BTreeMap` order (data-model.md) and exists
   for determinism in tests, not as a contract a client may read meaning into. There is no
   `nextCursor`: unlike `workspace/readDirectory`, whose page is capped at 1000 entries, the task
   set is bounded by what one developer started.
10. **Listing replaces no part of remembering.** FR-031d says what a client remembers across its
    own restart is its own business; a client that has its identities SHOULD attach to them
    directly rather than enumerate first, because a listing is a round trip that tells it what it
    already knew.

### Errors

| Code | Condition |
|---|---|
| `-32001` | `workspaceId` present, and unknown or unregistered |
| `-32007` | The listing itself exceeds §4.1's frame cap. The method is **unpaged** and §4.8 accepts that its result can (see below) |
| `-32601` | This engine predates `execution/list`. The client redeploys (§3.8, A-BOOT) |
| `-32602` | `workspaceId` present and not a string |

**`-32007` is the engine refusing its own answer, and it is reachable here and nowhere else in this
contract.** §4.8 states the arithmetic and accepts it: the method is unpaged, "unlike
`workspace/readDirectory`, which caps at a thousand entries and returns a cursor", so its result is
"bounded only by how many tasks one developer has started, and a large enough set would exceed
§4.1's frame cap and answer `-32007` against the engine's own listing. That is accepted because the
realistic count is tens, and recorded because the arithmetic does not care." It is carried in the
table for the same reason: a client that meets it cannot recover by retrying the identical call,
and the remedy — filter by `workspaceId`, which is the one parameter this method has — is only
obvious once the code is named. Guarantee 9's "there is no `nextCursor`" is the same fact from the
other side.

**`-32009` is not returned by this method**, and that is deliberate. A workspace whose root has
been deleted may still hold running tasks, and refusing to enumerate them would strand exactly
the processes FR-025 forbids stranding. The same reasoning applies to `workspace/close` below:
neither the call that finds a task nor the call that ends it may be blocked by the disappearance
of a directory.

---

## `workspace/close`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| result | — | null | — |

A workspace method, specified here because its entire obligation is executional: FR-024 and
SC-013 are this feature's, and §4.8 added the row on 2026-09-24 for them.

**§4.8 fixes the row, the tasks and the watches, and left three questions unanswered**: when
the response is written relative to the ends it causes, whether a second close is an error, and
whether the registration survives. They are answered by **A-WSCLOSE** in Appendix A, and
guarantees 3, 7 and 8 are this contract implementing that record rather than deciding anything of
its own. They were taken while this document was written, which left three decisions carrying
rejected alternatives outside Appendix A — a Principle III breach caught by analysis — and the
record closes it. The rejected alternative each guarantee names below is A-WSCLOSE's, restated
where the reader meets the rule.

### Guarantees

1. **Closing a workspace terminates its tasks** (FR-024, SC-013). Every task whose `workspace`
   field names this workspace is stopped; no task of any other workspace is touched. `Task`
   records its owning workspace at `runTask` and never changes it, which is what makes this a
   lookup rather than a search (data-model.md, `TaskSet::drain_for_workspace`).
2. **Termination is `SIGTERM` to the process group, then `SIGKILL` after 5 s** (plan.md, *Fixed
   Quantities*). To the **group**, so a shell's children go with it (FR-006a, FR-018, SC-012,
   SC-027). Five seconds is long enough for a build to flush and remove partial output, short
   enough that a developer who asked is not left waiting. This is the engine choosing a signal
   because no client named one — it is not `execution/terminate`, where FR-017 requires the
   caller to state the signal and the engine to send that one and no other.
3. **The response is written after every one of those tasks has been signalled** (A-WSCLOSE) — not
   after the last has **ended**, which is what this guarantee said in its first version. Ending
   takes up to guarantee 2's five-second escalation, and the use case runs on the engine's single
   dispatch thread, which is also the only reader of the client's stdin. Waiting there would mean
   five seconds in which no keystroke, resize or cancellation is so much as read off the pipe,
   which is §1.4 and FR-012 failing through the mechanism meant to satisfy FR-024. There is no
   deferred reply to fall back on: an `Action` is a reply, nothing, or a restart.

   SC-013 stays checkable without it. "Closing a workspace leaves zero of its tasks running" is
   observed through each task's `execution/onExit`, which is a defined event with a defined order,
   rather than through a response whose timing hid the wait: a test waits for N exits, not for a
   sleep. The escalations still run concurrently across the workspace's tasks, so closing ten
   costs about five seconds and not fifty, and a process blocked in `write` against a full
   retention buffer (FR-013) is the worst case and is still bounded — it cannot flush, so it
   reaches `SIGKILL` at five seconds.

   The alternative — answer immediately, having sent nothing — was rejected because it makes
   "closing a workspace leaves zero of its tasks running" true only eventually, and leaves a
   client no moment at which it may say the close has begun.
4. **It is deliberately not the same event as a dropped connection** (§4.8, A-TASKLIFE). A drop
   leaves tasks running, because a laptop moving between networks must not kill a build; a close
   is the developer saying they are done. Conflating the two would make the protocol unable to
   express the difference between an accident and an intention, and it is the reason this row
   exists rather than the transport's close being reused.
5. **The remaining output and the exits are delivered as ordinary notifications, and the
   identities are released after they are** (FR-022, FR-023). A terminated task is a task that
   ended: its last bytes precede its `onExit` exactly as they would for any other death, and its
   identity is free for reuse only once that exit has been delivered. A client that closes a
   workspace will therefore receive `onExit` for each of its tasks, and MUST NOT treat those as
   unexpected.
6. **It releases the workspace's watches** (§4.8) — F004's `WatchSet` for that workspace, emptied
   as the drop would empty it. That half is F004's contract, not this one's.
7. **It deregisters the workspace** (A-WSCLOSE). It is `workspace/register`'s counterpart, and a registration
   that survived a close would leave an id that resolves to a root the client has said it is done
   with. A later call naming that id is `-32001`, and the client's remedy is the one `-32001`
   always carries: register it again.
8. **Closing twice is `-32001`, and that is not the `terminate` case** (A-WSCLOSE). `execution/terminate` on
   an already-exited task succeeds (FR-019) because the client was racing an end the **engine**
   decided; a workspace never closes itself, so a second close is a client bug and reporting it
   is a service rather than a punishment. The two rules differ because the races differ, and a
   reader who expects idempotence here is expecting FR-019 to generalise further than it does.
9. **`-32009` is never returned.** Closing a workspace whose root has been deleted must work, or
   the tasks of a deleted directory can never be stopped — which is FR-025's abandoned process
   with a different cause. The root's existence is irrelevant to stopping a process and releasing
   a watch, and neither operation reads it.

### Errors

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` — including one already closed (guarantee 8) |
| `-32601` | This engine predates `workspace/close`. The client redeploys (§3.8, A-BOOT) |
| `-32602` | `workspaceId` absent or not a string |

---

## `execution/writeStdin`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `data` | base64 string (§4.8) | yes |

### Guarantees

1. **A notification has no response, so this method can report nothing** (§4.2). An unknown
   `taskId`, an exited task, a full buffer and a successful write are indistinguishable to the
   caller. This is a consequence of the catalogue's shape and it is stated rather than worked
   around: FR-014 requires the bytes to reach the process and gives the client no way to learn
   they did not.
2. **Bytes reach the process unmodified** (FR-014, SC-007). No line-ending translation, no
   encoding conversion, no trimming. What is written to the pseudo-terminal master is what was
   decoded from `data`.
3. **Writes for one task are applied in the order they arrive.** Two frames are two writes in
   frame order; there is no reordering and no coalescing of one client's input.
4. **An interrupt is not this method's job with `pty: true`, and is with `pty: false`.** FR-015
   requires an interrupt to reach the process as a signal rather than as literal text. With a
   pseudo-terminal, writing `0x03` is exactly that: the line discipline turns it into `SIGINT`
   delivered to the foreground process group, which is what a real terminal does and what makes
   FR-002 and FR-015 the same mechanism. With `pty: false` there is no line discipline, so `0x03`
   is a literal byte in the pipe and the interrupt must be `execution/terminate` with `SIGINT` —
   the signal plan.md fixes for an interrupt. **A client must branch on the shape it chose.** This
   follows from A-TASKSTREAM and is stated nowhere else.
5. **Input to a task that has exited is discarded, not queued.** There is no process to read it
   and no error to return (guarantee 1).
6. **`data` is base64** (§4.8). FR-014 requires arbitrary bytes and a JSON string cannot carry
   them. `workspace/readFile` sets the precedent with an explicit `encoding` of `utf8` or
   `base64`; execution has no such field and needs none, because unlike a file the answer is
   always the same — which is the reasoning §4.8 now gives for fixing it rather than offering it.

### Errors

None. A notification carries no `id`, and §4.2 gives it no response. A malformed frame is dropped.

**Implementation note, because the engine cannot do this today.** `dispatch` in
`engine/src/adapters/inbound/rpc.rs` reads `id` and returns `Action::Nothing` for any frame
without a **string** one, **before the method match is reached**. `execution/writeStdin` and
`execution/resizePty` are the **first client-to-engine notifications in the catalogue**, so
dispatch must gain an id-less path before either can be implemented. plan.md records this under
*Constraints* as **foundational work for this feature rather than part of any one story** — no
task that sends a keystroke can pass until dispatch can receive one. plan.md also notes, and does
not fix, that the same line uses `as_str()`, so a numeric id — legal under JSON-RPC 2.0 — is
dropped as though it were a notification; no client sends one.

---

## `execution/resizePty`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `cols` | integer, > 0 | yes |
| param | `rows` | integer, > 0 | yes |

### Guarantees

1. **Idempotent, and last-write-wins.** Resizing to the dimensions already in force changes
   nothing and is not an error. Two resizes in flight leave the later one in force.
2. **The process observes the change** (FR-016, SC-009), within 500 ms of the frame arriving. On
   a pseudo-terminal that is the window-size change and the `SIGWINCH` that follows it — the
   mechanism is named in exactly one file (runner-port.md).
3. **A resize for a `pty: false` task is silently ignored** (§4.8). There is no terminal to
   resize, and a notification has no way to refuse. It is not an error, and a client switching
   shapes must not have to special-case its panel code.
4. **A resize is never inferred.** Between the start and the first resize a process reads the size
   `runTask` was given in `cols` and `rows` — or, if the client omitted them, the 80 x 24 §4.8
   defaults to and plan.md fixes (`runTask` guarantee 7). The engine derives dimensions from
   nothing else: not from a previous task, not from an attaching client, and not from a default of
   its own — 80 x 24 is the specification's default, which is what makes applying it a rule rather
   than an inference.

### Errors

None, for `writeStdin`'s reason. `cols` or `rows` absent, zero or not an integer is a dropped
frame; the engine does not resize to zero, which some programs read as "no terminal".

---

## `execution/terminate`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `signal` | string — `"SIGINT"`, `"SIGTERM"` or `"SIGKILL"` | yes |
| result | — | null | — |

### Guarantees

1. **The signal is named by the caller and is the *initial* signal** (FR-017: the request "MUST
   state which signal it sends"; §4.8). The engine never substitutes a different first signal.
   Whether a second follows is fixed by §4.8 and follows from which one was named:

   - **`SIGTERM` escalates to `SIGKILL` after 5 s** (plan.md, *Fixed Quantities*), because "a stop
     that a process can decline is not a stop". Five seconds is long enough for a build to flush
     and remove partial output.
   - **`SIGINT` does not escalate.** It is the developer asking a foreground process to stop the
     way Ctrl-C asks, and a program legitimately handling it — a test runner printing a summary, a
     shell returning to its prompt — must not then be killed for having handled it. A client that
     wants the process gone asks for `SIGTERM`.
   - **`SIGKILL` has nothing to escalate to.**

   This is consistent with FR-017 rather than an exception to it: the request states the signal it
   **sends**, and the escalation is a published consequence of that choice rather than a
   substitution made behind the caller. The same rule ends a workspace's tasks, where no caller
   named anything and the engine therefore begins at `SIGTERM` (`workspace/close`, guarantee 2).
2. **The signal goes to the process group, not the process** (FR-006a, FR-018, SC-027). Stopping a
   build stops its compilers, at any depth, because every descendant inherits the group. Signalling
   the pid alone is the defect the spec's *A process that spawns children* edge case describes.
3. **Terminating an already-exited task succeeds** (FR-019). A client racing an exit is not told it
   did something wrong. The result is the same null result as a successful signal, and the client
   cannot tell the two apart — deliberately, because there is nothing it would do differently.
   §4.4 no longer contradicts this: `-32006` is "Task not found" and says in its own prose that
   stopping a task that has already stopped is a success, "since the caller asked for it not to be
   running and it is not running".
4. **The response says the signal was delivered, never that the process died.** The spec's *A
   process that ignores a request to stop* edge case is real: a process may catch `SIGTERM` and
   continue, and it has five seconds in which continuing is allowed. The exit, when it comes,
   comes as `onExit` — which is the only thing that says a task ended (FR-020). A client that
   sends `SIGINT` and needs the process gone regardless sends `SIGTERM` afterwards; a client that
   sent `SIGTERM` needs to send nothing, because the escalation is already running.
5. **`signal` is the signal's name, from a closed vocabulary of three** (§4.8). `SIGINT` is the
   interrupt (FR-015, and the signal plan.md fixes for one); `SIGTERM` asks; `SIGKILL` ends.
   A **name and not a number**, for §4.8's reason: signal numbers differ between platforms and the
   client is not always on the engine's, so a client on macOS composing a stop request should not
   have to know Linux's numbering — and an unrecognised name can be refused, whereas an
   unrecognised number is indistinguishable from a valid one. Anything else is `-32602`, which is
   `ResolvedPath`'s reasoning applied to a second kind of untrusted input reaching a syscall.
6. **Terminating does not release the identity.** Release follows the exit and its delivery
   (FR-023, guarantee 12 of `runTask`), so a client may terminate and then attach to collect the
   last output and the exit — which is the sequence the spec's *Output arriving after the process
   has exited* edge case requires to work.
7. **A task terminated by signal is reported as such** (FR-021, SC-010), distinguishably from one
   that exited with a code. The shape is task-events.md, guarantee 9.

### Errors

| Code | Condition |
|---|---|
| `-32006` | No live identity `taskId` — never started, or released. **Not** "already exited": see guarantee 3, and §4.4, which now says so itself |
| `-32602` | `signal` absent, or not one of `SIGINT`, `SIGTERM`, `SIGKILL` |

There is no `workspaceId` on this method, so `-32001` and `-32009` cannot be returned by it.

---

## The reattachment contract (FR-031b, FR-031d, SC-019, SC-023)

What a client does when a connection returns. It mirrors F004's reconnection contract and differs
in one way that matters: **the engine's task state survived and its watch set did not.**

```
connection returns
  → auth/handshake                         (resumed true or false; F002)
  → workspace/register                     (the registry did not survive; §4.8)
  → workspace/watch  <the whole set>       (F004; the set was emptied by the drop)
  → execution/list                         ONLY if the identities were lost      SC-023
  → execution/attach per remembered taskId FR-031b, FR-031d
      ↳ -32006        → the task is gone; tell the developer         FR-032
      ↳ running true  → it survived; retained output follows         SC-018, SC-019
      ↳ running false → exitCode or signal in the result,            SC-020
                        then the output, then onExit
  → execution/resizePty per attached task  (guarantee 9 of attach)
```

Four properties make this work without a discovery protocol for the ordinary case:

1. **The client usually already holds the identities**, because it chose them (§4.8, A-TASKLIFE).
   A client that restarted rather than reconnected reads them from its durable store — A-STATE's
   file (F000 `app-shell`), carrying the task identities **A-STATE2** adds to that payload.
   A-STATE2 supersedes A-STATE rather than editing it (spec Assumptions, FR-031d).
2. **A client that has lost them enumerates** (`execution/list`, SC-023). This is the exception,
   not the step: a client with its identities that lists first has spent a round trip learning
   what it knew.
3. **`attach` is idempotent and delivers nothing twice** (guarantee 7), so a client uncertain
   whether its attach landed may repeat it.
4. **The developer is told the outcome, not left to infer it** (FR-032, US5 scenario 5). Which
   tasks survived, how the finished ones finished, and what was missed all come from the per-task
   results above — which is why `running`, `retained`, `exitCode` and `signal` are in the result
   at all rather than being inferable from the stream that follows.

**A client that never returns leaves the task running** until it ends on its own or A-EC2's idle
stop takes the instance — thirty minutes after the disconnect, unless F005 decides a running task
defers it (A-TASKLIFE, second-order consequence). Nothing in this contract shortens that, and
`execution/list` does not change it: enumeration needs a connected client, and a client that never
returns never enumerates.

---

## Worked examples

Frames are snake_case and length-prefixed per §4.1; headers omitted.

**Starting a build with a terminal, sized** (US1, FR-001, FR-002, FR-016).

```json
{"jsonrpc":"2.0","id":"req_task_001","method":"execution/runTask",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-01",
           "command":["cargo","build","--release"],
           "cwd":".","env":{"RUST_LOG":"info"},"pty":true,"cols":132,"rows":43}}
```

```json
{"jsonrpc":"2.0","id":"req_task_001","result":{"pid":48211}}
```

`cargo` reads 132 columns at startup and lays its progress bar out to the panel the developer is
actually looking at. Omit `cols` and `rows` and it reads 80 x 24, which is a size somebody chose
and not the one on screen (guarantee 7).
Output follows as `execution/onStdout` (task-events.md); `onStderr` carries nothing for the life
of this task (A-TASKSTREAM, SC-028).

**Starting the same identity again while it is live** (FR-031c, SC-022). No second process.

```json
{"jsonrpc":"2.0","id":"req_task_002","method":"execution/runTask",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-01",
           "command":["cargo","build"],"cwd":".","env":{},"pty":true}}
```

```json
{"jsonrpc":"2.0","id":"req_task_002",
 "error":{"code":-32010,"message":"task identity is already running"}}
```

The correct response is `execution/attach`, not a retry — §4.4 says so, and a client that retries
in a loop against `-32010` is a client that will never reach the build it already has.

**A command that cannot be started** (FR-004, SC-015). No pid, no task, and zero `onExit` frames.

```json
{"jsonrpc":"2.0","id":"req_task_003","method":"execution/runTask",
 "params":{"workspace_id":"ws_7f2a","task_id":"test-02",
           "command":["cargo-nextest","run"],"cwd":".","env":{},"pty":false}}
```

```json
{"jsonrpc":"2.0","id":"req_task_003",
 "error":{"code":-32011,"message":"command could not be started",
          "data":{"reason":"not_found","program":"cargo-nextest"}}}
```

`data` carries the reason because §4.4 requires it for errors a user can act on, and the remedy
here is the developer's. It carries **no environment** (FR-005a, SC-025), on the path where a
diagnostic would most naturally quote the whole request.

**A `cwd` escaping the root — refused, and nothing starts** (FR-003, §4.7).

```json
{"jsonrpc":"2.0","id":"req_task_004","method":"execution/runTask",
 "params":{"workspace_id":"ws_7f2a","task_id":"shell-01",
           "command":["/bin/bash","-l"],"cwd":"../../etc","env":{},"pty":true}}
```

```json
{"jsonrpc":"2.0","id":"req_task_004",
 "error":{"code":-32002,"message":"path refused: outside the workspace root"}}
```

The refusal is identical whether or not `../../etc` exists (§4.7, F003's FR-007). No pid was
allocated, no identity became live, and `env` appears nowhere in the message (FR-005a, SC-025).

**Typing a line, then interrupting** (FR-014, FR-015, SC-007, SC-008). `data` is base64: `yes\n`
is `eWVzCg==`, and the single byte `0x03` is `Aw==`.

```json
{"jsonrpc":"2.0","method":"execution/writeStdin",
 "params":{"task_id":"build-01","data":"eWVzCg=="}}
```

```json
{"jsonrpc":"2.0","method":"execution/writeStdin",
 "params":{"task_id":"build-01","data":"Aw=="}}
```

The second frame is an interrupt **because this task has a pseudo-terminal**: the line discipline
turns `0x03` into `SIGINT` for the foreground process group. The identical frame sent to a
`pty: false` task is one literal byte in a pipe, and that task is interrupted with
`execution/terminate` carrying `"SIGINT"` instead (guarantee 4 of `writeStdin`).

**Resizing the panel** (FR-016, SC-009).

```json
{"jsonrpc":"2.0","method":"execution/resizePty",
 "params":{"task_id":"build-01","cols":132,"rows":43}}
```

**Reattaching after a twelve-minute disconnection to a build still running** (US5, SC-018,
SC-019).

```json
{"jsonrpc":"2.0","id":"req_task_005","method":"execution/attach",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-01"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_005",
 "result":{"pid":48211,"running":true,"retained":1310720}}
```

1 310 720 bytes are owed — under the 4 MiB the engine will hold before it slows the build
(plan.md) — and they arrive as `onStdout` frames after this response, in order, before anything
the build produces next (FR-031b). Neither `exit_code` nor `signal` is present, because the task
has not ended. The developer is told the build survived and how much they missed, from this
result rather than by watching a panel resume (FR-032).

**Reattaching to a task that ended while nobody was watching** (US5 scenario 3, SC-020).

```json
{"jsonrpc":"2.0","id":"req_task_006","method":"execution/attach",
 "params":{"workspace_id":"ws_7f2a","task_id":"test-02"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_006",
 "result":{"pid":48300,"running":false,"retained":8192,"exit_code":0}}
```

Eight kilobytes of output, then `execution/onExit`. Not an error, and `-32006` would be wrong:
the output and the exit are still owed to this client (spec edge case, *A client reattaching to a
task that has already exited*). The client can render "tests passed" from this response, before
the eight kilobytes have finished arriving — which is what carrying `exitCode` here buys, and
`running: false` alone would not.

**Attaching to an identity that was never started.**

```json
{"jsonrpc":"2.0","id":"req_task_007","method":"execution/attach",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-99"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_007",
 "error":{"code":-32006,"message":"no such task"}}
```

**A client that lost its identities, enumerating** (SC-023, FR-025). Fresh install; the profile
holding the task identities is gone, and two tasks are still running on the instance.

```json
{"jsonrpc":"2.0","id":"req_task_008","method":"execution/list","params":{}}
```

```json
{"jsonrpc":"2.0","id":"req_task_008",
 "result":{"tasks":[
   {"task_id":"build-01","workspace_id":"ws_7f2a",
    "command":["cargo","build","--release"],"pty":true,
    "pid":48211,"running":true},
   {"task_id":"test-02","workspace_id":"ws_7f2a",
    "command":["cargo","test"],"pty":false,
    "pid":48300,"running":false,"exit_code":0}]}}
```

`params` is `{}` rather than a `workspace_id`, because a client that lost its identities lost the
workspaces they belonged to as well. The developer is now shown two reachable tasks, one still
running and one finished, and can attach to either. Without this call both would run until
A-EC2's idle stop (FR-025). Note `test-02`: it has ended, and it is still listed, because its
exit has not been delivered and its identity is therefore still live (guarantee 4).

**Stopping a build, and stopping it again after it has gone** (FR-017, FR-019, SC-012, SC-027).

```json
{"jsonrpc":"2.0","id":"req_task_009","method":"execution/terminate",
 "params":{"task_id":"build-01","signal":"SIGTERM"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_009","result":null}
```

```json
{"jsonrpc":"2.0","id":"req_task_010","method":"execution/terminate",
 "params":{"task_id":"build-01","signal":"SIGKILL"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_010","result":null}
```

The second call **succeeds** although the task had already exited (FR-019), and the signal reached
a process group that no longer exists. A client racing an exit is not told it did something wrong.
The second call was also unnecessary: the `SIGTERM` was already escalating to `SIGKILL` on its own
five seconds after the first (guarantee 1). Sending it early is harmless and sending it at all is
the client's choice; what a client may **not** infer from the first result is that nothing further
will happen.

**Closing a workspace** (FR-024, SC-013).

```json
{"jsonrpc":"2.0","id":"req_ws_011","method":"workspace/close",
 "params":{"workspace_id":"ws_7f2a"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"build-01","signal":"SIGTERM"}}
```

```json
{"jsonrpc":"2.0","id":"req_ws_011","result":null}
```

`build-01` was still running; it was sent `SIGTERM` on its process group, it ended, and the null
result followed its end (guarantee 3). A scan of the process group at the moment that result
arrives finds nothing, which is SC-013. Had the build ignored `SIGTERM` it would have been killed
five seconds later and the result would have arrived then (plan.md). A second
`workspace/close` for `ws_7f2a` answers `-32001`.

---

## What a client may and may not infer

| From these methods, a client MAY infer | A client MUST NOT infer |
|---|---|
| That a returned `pid` names a live process at the moment of the reply | That it is still live now — the process may have exited before the frame was read |
| That a `runTask` error means nothing started (every error in the table fails the whole call) | That an error means the identity is free — `-32010` means the opposite |
| That `-32010` means the identity is running | That retrying will eventually work. The remedy is `attach` (§4.4) |
| That `-32011` means the command never ran | That the engine failed. It is the developer's `command` or `cwd`, and `data` says which |
| That `running: false` on attach means the task ended, and `exitCode` or `signal` says how | That the `onExit` notification will not also arrive — it does, after the output (FR-022) |
| That `retained` counts bytes owed to this attachment | That those bytes are in the result, or that they arrive in one frame (§4.1) |
| That an `execution/list` entry names a task it can attach to | That the task is still running now — a listing is a snapshot of what the engine knows (guarantee 3) |
| That an absent `taskId` in a full listing means the identity is free | That the task never existed — a released identity leaves no trace (FR-023) |
| That a `terminate` result means the signal was delivered | That the process is dead (guarantee 4), or that its children are — only `onExit` says the first, and only after it |
| That `terminate` sends exactly the signal named **first** | That nothing follows it — a `SIGTERM` becomes a `SIGKILL` five seconds later, and a `SIGINT` does not (guarantee 1) |
| That a `workspace/close` result means every one of that workspace's tasks has been **signalled** (guarantee 3) | That they have **ended** — a `SIGTERM` takes up to five seconds to become a `SIGKILL` — or that their `onExit` frames have already been delivered, or that the workspace is still registered |
| That a `writeStdin` frame was written to the pipe | That the process received it. A notification reports nothing, including failure (guarantee 1) |
| That a `pty: true` task's `0x03` is an interrupt (guarantee 4) | That the same holds with `pty: false`, where it is a literal byte |
| That `cols` and `rows` on `runTask` sized the terminal, and that omitting them yields 80 x 24 (guarantee 7) | That 80 x 24 is the panel's size — it is the specification's default, and a client whose panel differs still owes a `resizePty` |
| That `-32006` means no live identity | That the task never existed — it may have run, exited and been collected (FR-023) |
| That a dropped connection leaves tasks running (A-TASKLIFE) | That `workspace/close` does — it is the opposite event, and ends them (FR-024) |

---

## What the amendments settled

Recorded so that a reader comparing this contract against an older copy can see which
uncertainties were closed and by which document. Adding a code or describing an existing parameter
does not increment `protocolVersion` (§4.8), and none of these did.

| Was owed | Now stated in |
|---|---|
| A code for "the identity is already running" | §4.4, `-32010` |
| A code for "the command could not be started" | §4.4, `-32011` |
| `-32006` narrowed to "Task not found", so FR-019 and SC-020 stop contradicting it | §4.4, with the reasoning in its prose |
| `command` is argv, not a shell line | §4.8, Execution prose |
| `data` is base64 on `writeStdin`, `onStdout`, `onStderr` | §4.8, Execution prose |
| Task identities are engine-global | §4.8, Execution prose |
| A terminal's size at creation | §4.8, `runTask`'s `cols?` and `rows?` — **partly**; see below |
| Chunk size and time bounds, retention bound, resource limits, scrollback bound, signal vocabulary and escalation | plan.md, *Fixed Quantities* |
| A method that closes a workspace (FR-024, SC-013) | §4.8, `workspace/close` |
| A method that enumerates tasks (SC-023, FR-025) | §4.8, `execution/list`, element fields included |
| Whether a mismatched `workspaceId` on `attach` is serviced or refused | §4.8, `-32001` and its reasoning |
| Whether `terminate` escalates, and after which signal | §4.8 prose, with the 5 s in plan.md |

## What was open here, and is not any more

Both items this document raised have since been closed in the places they belonged, which is
recorded rather than deleted so that the reasoning survives the resolution.

1. **A `pty: true` task started without `cols` and `rows`** landed on the pseudo-terminal's own
   default, which on Linux is 0×0 — the one value `resizePty` refuses to set, because programs
   read it as "no terminal". This contract declined to invent a number, on FR-006b's principle
   that a quantity nobody chose is one nobody can defend when it fires. §4.8 now defaults the two
   to **80 × 24** and plan.md's *Fixed Quantities* carries the row, so the absent case has a value
   somebody chose and a reason attached to it.
2. **Nothing bounded what a task writes to disk.** `RLIMIT_FSIZE` was unset, and unlike CPU time
   and process count it had not been *declined* — it had simply not been considered, which is a
   different thing and the reason this was worth raising. plan.md now carries a **File size** row
   declining it explicitly: the limit caps a single file, a build legitimately writes large ones,
   and any value low enough to stop a runaway log is low enough to break real output. A task can
   still fill the volume; §5.5 and §16 accept that under single tenancy, and it is now an accepted
   consequence rather than an unexamined one.

## What is NOT here

**`execution/onStdout`, `onStderr` and `onExit`** — the three notifications, in task-events.md.

**The `TaskRunner` port** — runner-port.md. Nothing in this document names a pseudo-terminal
mechanism, and nothing may.

**What `workspace/close` does to watches.** It releases them (§4.8), and that half belongs to
F004's `WatchSet` and F004's contract. This document specifies only the tasks.

**Cancellation.** `$/cancelRequest` (§4.5) applies to requests in flight. Of the four requests
here only `workspace/close` takes measurable time, and cancelling it is meaningless: the signals
have been sent and the tasks are ending whether or not the caller is still waiting. `runTask`
returns when the process exists, and stopping a task is `terminate`, not a cancellation of the
request that started it.

**A way to ask whether one named task exists.** `execution/attach` answers it, at the cost of
attaching, and `execution/list` answers it without that cost. A third method that only tests
existence would be a way to probe identities without either, and nothing asks for one.
