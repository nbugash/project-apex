# Contract: Task Methods

**Feature**: F010 execution-terminals | **Date**: 2026-09-24

The five `execution/*` methods a client calls: three requests — `runTask`, `attach`, `terminate`
— and two notifications — `writeStdin`, `resizePty`. `project-apex-predator.md` §4.8 is the
source of truth for the catalogue; this document states the guarantees its table has no room for.

Unlike F004, four of the five rows already existed. **One did not: `execution/attach` was added to
§4.8 on 2026-09-24**, together with the prose defining what `pty` does. Both were verified present
in the catalogue before this contract was written, not assumed from the plan — §4.8 now carries an
`execution/attach` row returning `{pid, running, retained}` and a paragraph stating that with
`pty: true` output arrives merged on `onStdout` and `onStderr` carries nothing. The amendments this
contract still requires are listed at the end, and they are **not** applied.

Rationale for the shapes below is in research.md, *Attaching to a task that is already running*,
*What the `pty` parameter means, and what it costs* and *Backpressure comes for free*. It is not
restated here. The decisions are **A-TASKLIFE**, **A-TASKLIMIT** and **A-TASKSTREAM**.

---

## What is shared by every method here

**A task identity is chosen by the client** (FR-001, §4.8) and is a bare string. `runTask` and
`attach` carry `workspaceId`; `writeStdin`, `resizePty` and `terminate` do **not**, and neither
does any notification in task-events.md. The identity is therefore **engine-global, not scoped per
workspace**: nothing after the start can name a workspace, so nothing after the start can
disambiguate one. A client that starts `build` in two workspaces has named one task twice. See
*Amendments still required*, item 4.

**`workspaceId` on `runTask` and `attach` says where the task runs, and is validated as F003
validates it.** Unknown or unregistered is `-32001`; registered with a vanished root is `-32009`
(§4.4). `attach` with a `taskId` that is live under a **different** workspace is `-32006`, not a
cross-workspace success and not a distinct code: a caller able to tell "not yours" from "not
there" can enumerate another workspace's tasks from refusals alone, which is the reasoning
`PathRefusal::Refused` already applies to paths in `engine/src/domain/path.rs`.

**`cwd` is untrusted input and §4.7 is normative for it** (FR-003, Principle VI). §4.7's own
sentence names `relativePath`; `runTask`'s parameter is called `cwd`, and the check is identical
and mandatory regardless — the engine canonicalises, asserts containment, and refuses with
`-32002`. That an escape fails the **whole call** and starts no process is the same rule F004's
`workspace/watch` applies for the same reason: degrading a boundary check to a partial outcome
makes it advisory, which Principle VI forbids.

**Wire spelling is snake_case** (§4.8). `workspaceId` is `workspace_id`, `taskId` is `task_id`,
`exitCode` is `exit_code`; `command`, `cwd`, `env`, `pty`, `data`, `cols`, `rows`, `signal`,
`pid`, `running` and `retained` are single words already.

**Adding a method does not increment `protocolVersion`** (§4.8). An engine without
`execution/attach` answers `-32601`, which is exactly the signal a client needs to know it must
redeploy — F004's reasoning for `workspace/watch`, unchanged.

---

## `execution/runTask`

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `taskId` | string, client-chosen | yes |
| param | `command` | array of string — program, then arguments | yes |
| param | `cwd` | string, workspace-relative; `"."` is the root | yes |
| param | `env` | object, string to string | yes — possibly empty |
| param | `pty` | boolean | yes |
| result | `pid` | integer, host process id | yes |

### Guarantees

1. **`command` is an argv array, never a shell line.** The engine does not interpose `sh -c`. The
   spec is explicit that this feature is *not a shell* and does not "implement one, configure one,
   or assume one"; a string command would make the engine implement one, and would make quoting
   the engine's problem for input it must not interpret. A client that wants a shell names the
   shell as `command[0]` — which is what the spec's Assumptions mean by "what it starts is a
   shell — a command like any other". **§4.8 does not say which shape `command` is**; see
   *Amendments still required*, item 3.
2. **Starting is not idempotent, and that is the requirement.** A `taskId` whose identity is
   already live is refused, no second process is started, and the refusal is distinguishable from
   every other failure (FR-031c, SC-022, US5 scenario 4). Making `runTask` idempotent was
   considered and rejected in research.md, *Attaching to a task that is already running*: it makes
   the two outcomes indistinguishable at the call site, which is the failure FR-031c names.
   **§4.4 has no code for this condition**; see *Amendments still required*, item 1.
3. **A command that cannot be started fails the request, and never appears as a task that ran.**
   FR-004 and SC-015 admit no other outcome: reported as a start failure in 100% of exercised
   cases and as a task exiting in zero. So a spawn failure MUST NOT be answered with a `pid` and a
   following `onExit`. **§4.4 has no code for this condition either**; item 2.
4. **The environment is the engine's, with `env` applied over it** (spec Assumptions). Starting
   from empty would leave no `PATH`, and every task would fail for one reason. `env` overrides
   per key; it does not replace the set.
5. **`env` is never logged, never included in an error `message` or `data`, and never reaches a
   crash report** (FR-005a, SC-025) — **including when the task fails to start**, which is the
   path on which a diagnostic would most naturally quote the request. The obligation is structural
   at the port: see runner-port.md, guarantee T10.
6. **`pty` chooses one of two shapes and the choice is exclusive** (A-TASKSTREAM, FR-008,
   FR-008a). `true` gives a pseudo-terminal, `isatty` is true, and output arrives merged on
   `onStdout` with `onStderr` carrying nothing. `false` gives separate pipes, the two streams are
   distinguishable, and `isatty` is false. The consequences for delivery are task-events.md,
   guarantee 4.
7. **The task runs in its own process group** (FR-006a), which is what makes terminating it
   terminate what it spawned (FR-018, SC-027) rather than only the command named.
8. **The task runs under per-process resource limits** (FR-006, A-TASKLIMIT), inherited by its
   children. What those limits do **not** bound is stated in runner-port.md, *Resource limits*,
   and is recorded rather than closed.
9. **The task runs as the developer's own user, with no escalation** (FR-005, A-SEC, A-EC2).
10. **A returned `pid` means the process exists.** It is the host process id, the thing §15.2
    already says the engine tracks so a transient crash can be recovered. It is the process
    **group leader** by guarantee 7.
11. **The identity becomes live when this call succeeds and stays live until its exit has been
    delivered** (FR-023). Reuse before then is guarantee 2's refusal; reuse after it is a fresh
    task.
12. **A connection drop does not end the task** (A-TASKLIFE, FR-031, SC-018). This is a deliberate
    divergence from F004, where dropping the connection empties the watch set
    (watch-methods.md, `workspace/watch` guarantee 12), and from §7.3, which stops language
    servers on disconnect. A-TASKLIFE gives the reason: the developer asked for the build and did
    not ask for the language server.

### Errors

Each fails the whole call. No process is started and no identity becomes live.

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` (§4.4). The client registers and retries |
| `-32009` | Registered, and the root no longer exists (§4.4). The client tells the developer; it does **not** re-register |
| `-32002` | `cwd` escapes the workspace root, lexically or after a symlink resolves (§4.7, FR-003) |
| `-32003` | `cwd` is inside the root and is not there, or is not a directory |
| `-32602` | `command` absent, not an array, or empty; `env` not an object of strings; `pty` not a boolean; `taskId` absent or empty |
| **none yet** | **The identity is already live** (FR-031c, SC-022) — §4.4 has no code. Item 1 |
| **none yet** | **The command could not be started** (FR-004, SC-015) — §4.4 has no code. Item 2 |

`-32006` is **not** returned by this method. It means an identity that is not live, which for
`runTask` is the success precondition rather than a failure.

---

## `execution/attach` — **ADDED TO §4.8 ON 2026-09-24**

| | Name | Type | Required |
|---|---|---|---|
| param | `workspaceId` | string | yes |
| param | `taskId` | string | yes |
| result | `pid` | integer | yes |
| result | `running` | boolean | yes |
| result | `retained` | integer, bytes | yes |

### Guarantees

1. **Attaching is a different call from starting, and that is the entire point** (FR-031c). A
   client that has reconnected and is not yet certain what survived calls this; a client that
   means to start calls `runTask`. Neither can silently become the other.
2. **`retained` is a byte count, not the bytes.** The output produced while no client was attached
   is delivered afterwards as ordinary `onStdout`/`onStderr` notifications, chunked exactly as
   live output is, and `retained` says how much is owed.

   **Not fixed by §4.8 or by research.md.** The catalogue names the field and not its type;
   research.md writes "the output retained since it was last read", which reads as the bytes.
   A count is chosen here for three reasons, and the first is decisive. The retention bound
   (FR-013a) is larger than §4.1's 1 MiB frame cap, so the bytes cannot fit in a result at all —
   they would have to be chunked, and chunking is defined for notifications and not for responses.
   Second, FR-031b requires the missed output "in order, before any output produced after", which
   the one writer gives for free across notifications and would have to be re-established across a
   response boundary. Third, FR-022 requires output before the exit report, and an exit carried in
   this result would arrive **before** the output it followed. A client that needs the bytes
   enumerated does not need a different shape; it needs to read its notifications.
3. **Delivery order after a successful attach is fixed** (FR-031b, FR-022, SC-019, SC-020):
   everything retained, in the order produced, then anything produced since, then — if the task
   has ended — `execution/onExit`. Nothing produced after the attach overtakes anything retained.
4. **A task that has already exited attaches successfully.** `running` is `false`, `retained` is
   what is still owed, and the exit follows as `onExit` once that output has been delivered
   (FR-031b, US5 scenario 3, SC-020). **Attaching to an exited task is not an error**, which is
   the reading of `-32006` this contract requires and §4.4's wording currently contradicts — item
   5 below.
5. **An identity that was never started, or whose exit has already been delivered and identity
   released (FR-023), is `-32006`.** These two are one condition on purpose: after release the
   identity carries no history, so "never started" and "finished and collected" are the same fact.
6. **Attaching twice is not an error and delivers nothing twice.** A client that re-attaches
   having already received the retained output receives `retained: 0`. The engine tracks what an
   attachment has been sent, not what exists.
7. **`pid` is returned for a task that has exited.** It is the id the process had. §15.2's PID
   mapping is what makes reattachment after an engine restart possible at all, and a client that
   cannot see the pid of a task it is being told about cannot reconcile with the host.
8. **Attaching neither starts, stops, resizes nor writes.** It has no side effect on the process.
   In particular it does not resize the pseudo-terminal to the reattaching panel's dimensions;
   that is a `resizePty` the client sends itself, and a client that forgets to leaves the process
   laying out to the old width.

### Errors

| Code | Condition |
|---|---|
| `-32001` | Unknown or unregistered `workspaceId` |
| `-32009` | Registered, and the root no longer exists |
| `-32006` | No live identity `taskId` in this workspace — never started, already released, or live under a different workspace (guarantee 5, and *What is shared* above) |
| `-32601` | This engine predates `execution/attach`. The client redeploys (§3.8, A-BOOT) |
| `-32602` | `taskId` absent or not a string |

---

## `execution/writeStdin`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `data` | base64 string — **see item 6** | yes |

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
   is a literal byte in the pipe and the interrupt must be `execution/terminate` with `SIGINT`.
   **A client must branch on the shape it chose.** This follows from A-TASKSTREAM and is stated
   nowhere else.
5. **Input to a task that has exited is discarded, not queued.** There is no process to read it
   and no error to return (guarantee 1).
6. **`data` is base64.** FR-014 requires arbitrary bytes and a JSON string cannot carry them.
   **§4.8 does not say this**, for `writeStdin` or for either output notification — item 6 below.
   `workspace/readFile` already sets the precedent with an explicit `encoding` of `utf8` or
   `base64`; execution has no such field and needs none, because unlike a file the answer is
   always the same.

### Errors

None. A notification carries no `id`, and §4.2 gives it no response. A malformed frame is dropped.

**Implementation note, because the engine cannot do this today.** `dispatch` in
`engine/src/adapters/inbound/rpc.rs` returns `Action::Nothing` for any frame without a string
`id`, before the method match is reached. `execution/writeStdin` and `execution/resizePty` are the
**first client-to-engine notifications in the catalogue**, so dispatch must gain an id-less path
before either can be implemented. Recorded here rather than discovered in a task; see the report
of findings.

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
3. **A resize for a `pty: false` task is accepted and does nothing.** There is no terminal to
   resize. It is not an error, because a notification has none (guarantee 1 of `writeStdin`), and
   a client switching shapes must not have to special-case its panel code.
4. **A resize is never inferred.** The engine does not size a task's terminal from anything but
   this notification, and §4.8's `runTask` carries no dimensions — so between start and the first
   resize a process reads whatever the engine created the terminal with. That initial size is a
   value the plan owes (item 7 below), and a process that reads its width before the first resize
   arrives reads that value.

### Errors

None, for `writeStdin`'s reason. `cols` or `rows` absent, zero or not an integer is a dropped
frame; the engine does not resize to zero, which some programs read as "no terminal".

---

## `execution/terminate`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `signal` | string, closed vocabulary — **item 8** | yes |
| result | — | null | — |

### Guarantees

1. **The signal is named by the caller, not chosen by the engine** (FR-017: the request "MUST
   state which signal it sends"). The engine does not substitute one, and does not escalate on its
   own within a single call.
2. **The signal goes to the process group, not the process** (FR-006a, FR-018, SC-027). Stopping a
   build stops its compilers, at any depth, because every descendant inherits the group. Signalling
   the pid alone is the defect the spec's *A process that spawns children* edge case describes.
3. **Terminating an already-exited task succeeds** (FR-019). A client racing an exit is not told it
   did something wrong. The result is the same null result as a successful signal, and the client
   cannot tell the two apart — deliberately, because there is nothing it would do differently.

   **This contradicts §4.4's wording for `-32006`**, which reads "Task not found **or already
   exited**". Both halves cannot be an error while FR-019 requires success and SC-020 requires an
   exited task to be attachable. Item 5 below narrows it.
4. **Terminating is a request, not a guarantee, until it is `SIGKILL`.** The spec's *A process
   that ignores a request to stop* edge case is real: a process may catch `SIGTERM` and continue.
   The response says the signal was delivered, never that the process died. The exit, when it
   comes, comes as `onExit` — which is the only thing that says a task ended (FR-020).
5. **Escalation is the client's, or the plan's, and this contract adds none.** The spec makes "the
   escalation after a process ignores one" a plan-level decision; plan.md does not yet make it
   (item 8). Nothing here sends a second signal after a delay.
6. **Terminating does not release the identity.** Release follows the exit and its delivery
   (FR-023, guarantee 11 of `runTask`), so a client may terminate and then attach to collect the
   last output and the exit — which is the sequence the spec's *Output arriving after the process
   has exited* edge case requires to work.
7. **A task terminated by signal is reported as such** (FR-021, SC-010), distinguishably from one
   that exited with a code. The shape is task-events.md, guarantee 9.

### Errors

| Code | Condition |
|---|---|
| `-32006` | No live identity `taskId` — **never started, or released**. Not "already exited": see guarantee 3 and item 5 |
| `-32602` | `signal` absent, or not in the closed vocabulary the plan fixes (item 8) |

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
  → execution/attach per remembered taskId FR-031b, FR-031d
      ↳ -32006        → the task is gone; tell the developer         FR-032
      ↳ running true  → it survived; retained output follows         SC-018, SC-019
      ↳ running false → it ended while away; output then onExit      SC-020
  → execution/resizePty per attached task  (guarantee 8 of attach)
```

Three properties make this work without a discovery protocol:

1. **The client already holds the identities**, because it chose them (§4.8, A-TASKLIFE). A client
   that restarted rather than reconnected reads them from the session state F002 already persists
   (spec Assumptions, FR-031d).
2. **`attach` is idempotent and delivers nothing twice** (guarantee 6), so a client uncertain
   whether its attach landed may repeat it.
3. **The developer is told the outcome, not left to infer it** (FR-032, US5 scenario 5). Which
   tasks survived and what was missed comes from the per-task results above, which is why
   `running` and `retained` are in the result at all rather than being inferable from the stream
   that follows.

**A client that never returns leaves the task running** until it ends on its own or A-EC2's idle
stop takes the instance — thirty minutes after the disconnect, unless F005 decides a running task
defers it (A-TASKLIFE, second-order consequence). Nothing in this contract shortens that.

---

## Worked examples

Frames are snake_case and length-prefixed per §4.1; headers omitted.

**Starting a build with a terminal** (US1, FR-001, FR-002).

```json
{"jsonrpc":"2.0","id":"req_task_001","method":"execution/runTask",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-01",
           "command":["cargo","build","--release"],
           "cwd":".","env":{"RUST_LOG":"info"},"pty":true}}
```

```json
{"jsonrpc":"2.0","id":"req_task_001","result":{"pid":48211}}
```

Output follows as `execution/onStdout` (task-events.md). `onStderr` carries nothing for the life
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

**`-32010` does not exist in §4.4.** The frame above is what this contract requires and what the
amendment in item 1 must create; an engine built today has no code to put there.

**A `cwd` escaping the root — refused, and nothing starts** (FR-003, §4.7).

```json
{"jsonrpc":"2.0","id":"req_task_003","method":"execution/runTask",
 "params":{"workspace_id":"ws_7f2a","task_id":"shell-01",
           "command":["/bin/bash","-l"],"cwd":"../../etc","env":{},"pty":true}}
```

```json
{"jsonrpc":"2.0","id":"req_task_003",
 "error":{"code":-32002,"message":"path refused: outside the workspace root"}}
```

The refusal is identical whether or not `../../etc` exists (§4.7, F003's FR-007). No pid was
allocated, no identity became live, and `env` appears nowhere in the message (FR-005a, SC-025).

**Typing a line, then interrupting** (FR-014, FR-015, SC-007, SC-008). `data` is base64: `yes\n`
is `eWVzCg==`, and `0x03` is `Aw==`.

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
`pty: false` task is three literal bytes in a pipe, and that task is interrupted with
`execution/terminate` instead (guarantee 4 of `writeStdin`).

**Resizing the panel** (FR-016, SC-009).

```json
{"jsonrpc":"2.0","method":"execution/resizePty",
 "params":{"task_id":"build-01","cols":132,"rows":43}}
```

**Reattaching after a twelve-minute disconnection to a build still running** (US5, SC-018,
SC-019).

```json
{"jsonrpc":"2.0","id":"req_task_004","method":"execution/attach",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-01"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_004",
 "result":{"pid":48211,"running":true,"retained":1310720}}
```

1 310 720 bytes are owed and arrive as `onStdout` frames, in order, before anything the build
produces after this response (FR-031b). The developer is told the build survived and how much they
missed, from this result rather than by watching a panel resume (FR-032).

**Reattaching to a task that ended while nobody was watching** (US5 scenario 3, SC-020).

```json
{"jsonrpc":"2.0","id":"req_task_005","method":"execution/attach",
 "params":{"workspace_id":"ws_7f2a","task_id":"test-02"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_005",
 "result":{"pid":48300,"running":false,"retained":8192}}
```

Eight kilobytes of output, then `execution/onExit`. Not an error, and `-32006` would be wrong:
the output and the exit are still owed to this client (spec edge case, *A client reattaching to a
task that has already exited*).

**Attaching to an identity that was never started.**

```json
{"jsonrpc":"2.0","id":"req_task_006","method":"execution/attach",
 "params":{"workspace_id":"ws_7f2a","task_id":"build-99"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_006",
 "error":{"code":-32006,"message":"no such task"}}
```

**Stopping a build, and stopping it again after it has gone** (FR-017, FR-019, SC-012, SC-027).

```json
{"jsonrpc":"2.0","id":"req_task_007","method":"execution/terminate",
 "params":{"task_id":"build-01","signal":"SIGTERM"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_007","result":null}
```

```json
{"jsonrpc":"2.0","id":"req_task_008","method":"execution/terminate",
 "params":{"task_id":"build-01","signal":"SIGKILL"}}
```

```json
{"jsonrpc":"2.0","id":"req_task_008","result":null}
```

The second call **succeeds** although the task had already exited (FR-019), and the signal reached
a process group that no longer exists. A client racing an exit is not told it did something wrong.
Under §4.4's current wording for `-32006` an implementer would return an error here; that wording
is item 5.

---

## What a client may and may not infer

| From these methods, a client MAY infer | A client MUST NOT infer |
|---|---|
| That a returned `pid` names a live process at the moment of the reply | That it is still live now — the process may have exited before the frame was read |
| That a `runTask` error means nothing started (every error in the table fails the whole call) | That an error means the identity is free — a "already live" refusal means the opposite |
| That `running: false` on attach means the task ended | How it ended. The exit arrives as `onExit`, after the output that preceded it (FR-022) |
| That `retained` counts bytes owed to this attachment | That those bytes are in the result, or that they arrive in one frame (§4.1) |
| That a `terminate` result means the signal was delivered | That the process is dead (guarantee 4), or that its children are — only `onExit` says the first, and only after it |
| That a `writeStdin` frame was written to the pipe | That the process received it. A notification reports nothing, including failure (guarantee 1) |
| That a `pty: true` task's `0x03` is an interrupt (guarantee 4) | That the same holds with `pty: false`, where it is a literal byte |
| That `-32006` means no live identity | That the task never existed — it may have run, exited and been collected (FR-023) |
| That a dropped connection leaves tasks running (A-TASKLIFE) | That it leaves watches established — F004's set is emptied by the same drop |

---

## Amendments still required

**None of these is applied.** Each is stated here rather than assumed, so that an implementer
checking §4.4 or §4.8 and finding nothing knows it is a gap and not an oversight in this contract.
Adding a code or describing an existing parameter does not increment `protocolVersion` (§4.8).

1. **§4.4 needs a code for "task identity is already running."** FR-031c and SC-022 require the
   refusal to be distinguishable, and the closest existing code, `-32006`, means the exact
   opposite. `-32602` would conflate a live task with a malformed frame. The proposed row is
   `-32010 | Task identity is already running — refused rather than starting a second process`.
   `-32000` is taken by `session/restart` (§4.8 prose, and `wire::codes::RESTARTING`), so `-32010`
   is the next free value.
2. **§4.4 needs a code for "the command could not be started."** FR-004 and SC-015 require it to
   be distinguishable from a task that started and exited. `-32003` is reserved for a path inside
   the workspace root and a program name resolved through `PATH` is not one. Proposed:
   `-32011 | Task could not be started`, carrying a `data` object with the underlying reason
   (§4.4 requires `data` for errors a user can act on) **and not the environment** (FR-005a).
3. **§4.8 must say whether `command` is an argv array or a shell line.** Guarantee 1 reads it as
   argv; the catalogue's table says only `command`. A third party implementing from the table
   would guess, and the two guesses differ in whether the engine runs a shell.
4. **§4.8 should state that task identities are engine-global.** Only `runTask` and `attach` carry
   `workspaceId`; the other three methods and all three notifications address a bare `taskId`, so
   the identity cannot be per-workspace. Either the catalogue says so, or `workspaceId` is added
   to the remaining rows. This contract assumes the first because it is what the existing rows
   already imply, but it is an assumption about a gap, not a reading of a statement.
5. **§4.4's `-32006` must be narrowed to "Task not found."** Its current wording, "Task not found
   **or already exited**", contradicts FR-019 (terminate on an exited task succeeds) and SC-020
   (an exited task is attachable and reports its exit). "Already exited" is never an error
   condition for any method in this feature while the identity is live.
6. **§4.8 must state that `data` is base64**, on `writeStdin`, `onStdout` and `onStderr`. FR-009
   and SC-003 require arbitrary bytes including non-UTF-8, and a JSON string cannot carry them.
   The precedent is `workspace/readFile`'s explicit `encoding` field.
7. **§4.8's `runTask` has no terminal dimensions**, so a `pty: true` task is created at a size
   nobody chose and a process reading its width before the first `resizePty` reads that size.
   Either `cols?` and `rows?` are added to `runTask`, or the plan fixes an initial size as a
   stated value. FR-016 covers the resize and nothing covers the first moment.
8. **plan.md owes five quantities that four requirements make mandatory**, and this contract is
   parametric in all five rather than inventing them: the chunk size bound and the chunk time
   bound (research.md, *Chunking, ordering, and what is pure*, says both are "values fixed in the
   plan"); the retention bound (FR-013a); the per-process resource limit values (FR-006b); the
   panel's scrollback bound (FR-029a); and the signal vocabulary for `terminate` with the
   escalation after a process ignores one (spec Assumptions, "plan-level decisions"). plan.md
   states none of them today.

## What is NOT here

**`execution/onStdout`, `onStderr` and `onExit`** — the three notifications, in task-events.md.

**The `TaskRunner` port** — runner-port.md. Nothing in this document names a pseudo-terminal
mechanism, and nothing may.

**A method to close a workspace.** FR-024 and SC-013 require closing a workspace to terminate its
tasks, and §4.8 has `workspace/register` with no counterpart. There is no frame a client can send
that means "I am done with this workspace", so FR-024 is currently reachable only through
`session/shutdown` or the engine exiting — and a connection drop explicitly must **not** trigger
it (A-TASKLIFE). Stated here because a reader looking for the call that satisfies FR-024 will not
find one, and that is a gap in the catalogue rather than in this contract.

**A list-my-tasks method.** FR-031d requires a client that restarted to reach the tasks it
started, and the spec answers it with what F002 already persists rather than with a protocol
addition: "what it remembers across its own restart is its own business". A discovery method would
be a second answer to a question already answered, and research.md's reversal condition for
`execution/attach` — engine-assigned identities — is the only thing that would make one necessary.

**Cancellation.** `$/cancelRequest` (§4.5) applies to requests in flight. None of the three
requests here is long-running: `runTask` returns when the process exists, and stopping a task is
`terminate`, not a cancellation of the request that started it.
