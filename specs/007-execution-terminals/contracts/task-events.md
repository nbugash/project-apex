# Contract: Task Event Notifications

**Feature**: F010 execution-terminals | **Date**: 2026-09-24

The three `execution/*` notifications this feature implements. All three already exist in §4.8 as
rows; none has ever been specified beyond its parameter names, and this document is where the
guarantees behind them are written down. It also marks the places the catalogue must still be
amended.

Notifications carry no `id` and expect no response (§4.2). These three are engine-originated:
nothing in `dispatch` produces them, nothing correlates them, and every one of them is written
through the single `FrameWriter` F004 introduced —
`engine/src/adapters/outbound/frame_writer.rs`, which holds the output mutex for exactly one
frame. F010 is the first feature to put real volume through that seam, which is what makes
FR-012's obligation a measurement of the seam rather than of this feature alone.

The values this contract is parametric in — the chunk size bound, the chunk time bound and the
retention bound — are **owed by plan.md** and are not invented here. research.md, *Chunking,
ordering, and what is pure*, says both chunk bounds are "values fixed in the plan"; FR-013a says
the same of the retention bound; plan.md states none of the three. See task-methods.md,
*Amendments still required*, item 8.

Rationale for the shapes below is in research.md, *What the `pty` parameter means, and what it
costs*, *Chunking, ordering, and what is pure* and *Backpressure comes for free*. It is not
restated. The decisions are **A-TASKSTREAM**, **A-TASKLIFE** and **A-TASKLIMIT**.

---

## `execution/onStdout` and `execution/onStderr`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `data` | base64 string — **guarantee 1** | yes |

Wire spelling is snake_case: `task_id`, `data`.

The two notifications carry the identical shape and differ only in which stream they name. Which
of them a task ever uses is decided once, by `pty`, at `runTask` (guarantee 4).

---

## `execution/onExit`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `exitCode` | integer or null — **guarantee 9** | yes, per §4.8's row |
| param | `signal` | string, signal name | **exactly when** the task was killed by a signal |

Wire spelling: `task_id`, `exit_code`, `signal`.

---

## Guarantees

1. **Output is bytes, not text, and `data` is base64** (FR-009, SC-003). A process writes bytes;
   a JSON string holds Unicode scalar values, and the two are not the same set. A build emitting
   a filename in a non-UTF-8 locale, a `grep` over a binary, or a progress bar drawn with a
   private-use glyph would all be mangled into replacement characters by a lossy conversion — and
   SC-003 asserts **zero substitutions**. Base64 is the only encoding in the catalogue that
   carries arbitrary bytes, and `workspace/readFile` already uses it for exactly this reason.

   **§4.8 does not say so.** The rows name `data` and give it no encoding. This is the single most
   important thing the catalogue is missing for this feature, because an implementation that reads
   "data" as "a string of the output" passes every test written with ASCII fixtures and fails
   SC-003 the first time a real build prints a byte above 0x7F. task-methods.md, item 6.

2. **Every byte the process wrote is delivered, unmodified, exactly once** (FR-009, FR-013,
   SC-003, SC-021). Nothing is dropped, nothing is deduplicated, nothing is normalised. ANSI
   escape sequences, carriage returns, backspaces, bare `\r` progress redraws and NUL bytes all
   survive, because the engine does not interpret any of them — the panel does (FR-027).

3. **Output for one task arrives in the order the process produced it** (FR-010, SC-004). This is
   a per-task guarantee. Across tasks, nothing is promised beyond frame order on the one pipe
   (§4.6): two tasks writing at the same instant may interleave their frames in either order, and
   no client behaviour may depend on which. Within a task the guarantee is absolute, including
   across a chunk boundary, across a reattachment, and across the exit.

4. **A-TASKSTREAM: with `pty: true`, `onStderr` carries nothing, for the life of the task.**
   A pseudo-terminal is **one device**. A process whose standard output and standard error are
   both attached to it writes both into the same stream, which is what a terminal is and why
   `2>/dev/null` exists. Everything the process writes on either descriptor arrives on
   `execution/onStdout`, interleaved exactly as a shell interleaves them. SC-028 asserts it in
   both directions: `isatty` true with **zero** bytes on the error stream, and the same command
   with `pty: false` reporting false and delivering its error output separately.

   This is not a delivery policy that could be relaxed. There is no second stream to deliver;
   the kernel merged them before the engine saw a byte. A client that shows an empty "stderr"
   pane for a `pty: true` task is displaying the absence of a thing that does not exist, and
   should not offer the pane at all.

   §4.8 now states this in prose, added 2026-09-24 with the `execution/attach` row.

5. **No single notification exceeds §4.1's frame cap, and the chunk bound is well below it**
   (FR-011, SC-005). The arithmetic is worth stating because the obvious reading is wrong: the
   chunk bound is **not** 1 MiB.

   Base64 expands by 4/3. A raw chunk of `N` bytes becomes `4 × ceil(N / 3)` characters, plus the
   JSON-RPC envelope — `jsonrpc`, `method`, `task_id`, the `params` object, quoting — and the
   `Content-Length` header is outside the body but the cap applies to the frame. So the hard
   ceiling on a raw chunk is `3/4 × (1 MiB − envelope)` — around 786 200 bytes for a two-hundred
   byte envelope — and the plan's chosen value must sit below it with room. A chunk bound set to
   1 MiB overflows the cap by a third and produces `-32007` against the engine's own output.

   SC-005's 4 MiB unbroken line is the case this exists for: there is no newline to chunk on, so
   the bound is a byte count and nothing else. Line-based chunking was considered and rejected in
   research.md — a progress bar redrawing with carriage returns emits no newline for minutes.

6. **A chunk is emitted on whichever bound is reached first — size or time** (research.md,
   *Chunking, ordering, and what is pure*). Size alone starves an interactive process: a shell
   printing a prompt and waiting writes far less than a chunk, and the developer would see nothing
   until they typed. Time alone produces frames of unbounded size. The time bound is what makes
   SC-001's 500 ms achievable for a process that writes a little and stops.

   Both bounds are pure application code, fed bytes and told the time by the `Clock` port F004
   added (runner-port.md, *The dividing line*). Neither is a property of the pseudo-terminal, and
   neither needs a process to test.

7. **Output produced before an exit is delivered before the exit is reported** (FR-022, SC-011).
   The last lines of a failing build are not lost to the report of its failure. This is the spec's
   *Output arriving after the process has exited* edge case, and it is a real ordering hazard
   rather than a restatement of guarantee 3: a process can exit while bytes it wrote are still in
   the kernel's buffer, and a reaper that notices the exit first would report it ahead of them.
   The obligation reaches the port: `read` returns `Ended` only after every byte has been returned
   (runner-port.md, guarantee T4).

8. **An exit is reported exactly once to each attachment that has not already received it**
   (FR-020, SC-010, SC-020). A client attached when the task ends is told then. A client that was
   away is told when it attaches, after the retained output (task-methods.md, `execution/attach`
   guarantee 3). The exit is not re-sent to an attachment that already has it, and re-attaching
   after collecting it yields `-32006` — the identity was released (FR-023).

9. **A signal death is a distinct state, not a conventional exit code** (FR-021, SC-010). The
   spec's Key Entities are explicit: an exit is "a code, or a signal. **Distinct states, not one
   field with a convention**". So `signal` is present exactly when the task was killed by one, and
   `exit_code` is `null` in that case. A client branches on `signal` being present. It MUST NOT
   read `128 + n`, which is a shell's convention for reporting a signal through a single integer
   and is precisely the one-field-with-a-convention the spec rejects — and which would make a
   process that genuinely exits with code 130 indistinguishable from one killed by `SIGINT`.

   **§4.8's row contradicts this and must be amended.** It reads
   `taskId`, `exitCode`, `signal?` — `exitCode` mandatory, `signal` optional — which has no
   representation for "killed, therefore no code" other than a convention. The row should read
   `exitCode?`, `signal?`, with **exactly one present**. See *Amendments still required*.

10. **Output delivery does not delay interactive traffic** (FR-012, §4.6, Principle V, SC-006).
    The reader thread holds the frame writer's mutex for one frame at a time and releases it —
    which is what the writer was built for, and which makes this a claim about how long the lock
    is held rather than about how fast the reader is. §4.6's outbound priority queue puts editor
    and LSP traffic ahead of a build's output. SC-006 measures it under 50 MiB and **prints the
    measurement rather than asserting a threshold** (A-NFR).

    This is a measurement obligation, not a comment. A terminal is the highest-volume producer the
    channel will ever carry, and it is the failure mode the whole architecture exists to prevent
    (§1.5, US4).

11. **A producer that outruns the link is slowed; nothing is ever dropped** (FR-013, SC-021). When
    retained output reaches its bound the reader stops reading, the pseudo-terminal's kernel
    buffer fills, and the process's next write blocks — which is what happens to any program
    writing to a terminal nobody is reading. Nothing else is done, and that it needs no mechanism
    is the point (research.md, *Backpressure comes for free*).

    The consequence a client must understand: **a gap in time is not a gap in output.** Frames
    stopping for ten seconds means the developer's link is slow, not that anything was lost. There
    is no drop marker, no gap indicator and no truncation notice, because there is nothing to
    mark.

12. **A detached task behaves identically** (FR-031a, SC-021). Output produced while no client is
    attached is retained under the same bound, and a task that outruns it while detached is slowed
    exactly as an attached one would be. A process must not discover it is unobserved by being
    treated differently, and a build that runs faster when nobody is watching is a build whose
    timings mean nothing.

13. **Retained output is delivered in order, before anything produced since** (FR-031b, SC-019).
    On attach: everything retained, then everything new, then — if it has ended — the exit. Zero
    loss and zero reordering. The delivery is ordinary `onStdout`/`onStderr` frames, chunked by
    the same bounds; nothing distinguishes a retained chunk from a live one, and nothing needs to.

14. **No notification is delivered for a task the client is not attached to.** Starting a task
    attaches the client that started it; `execution/attach` attaches one that did not. A client
    that has neither started nor attached an identity receives nothing about it.

15. **The identity is released after the exit has been delivered** (FR-023, SC-014). Until then it
    is live and cannot be reused (task-methods.md, `runTask` guarantee 2). After it, `attach`
    answers `-32006` and `runTask` starts a fresh task. SC-014's hundred cycles assert that live
    identities and running processes both return to their starting count, which is the leak this
    guarantee prevents.

---

## What a client may and may not infer

Stated as a table because every row is a way an implementation has gone wrong before.

| From these notifications, a client MAY infer | A client MUST NOT infer |
|---|---|
| That the bytes in `data` are exactly what the process wrote (guarantee 2) | That they are text, or valid UTF-8, or safe to decode as a string (FR-009, SC-003) |
| That chunk boundaries are where the engine ended a frame | That they are meaningful — a boundary may fall mid-character, mid-escape-sequence or mid-line. A panel must buffer across frames |
| That output for one task is in order (guarantee 3) | Any ordering between two tasks (§4.6) |
| That an `onStdout` frame for a `pty: true` task may hold what the process wrote to either descriptor (guarantee 4) | That `onStderr` will ever arrive for it, or that a silent error stream means no errors (SC-028) |
| That a pause in frames means the link or the retention bound is holding the process (guarantee 11) | That output was dropped, or that the task is hung. There is no drop, so there is no marker |
| That `onExit` means the task ended and its output is complete (guarantee 7) | That the process's children ended — only `terminate` on the group does that (FR-018, SC-027) |
| That `signal` present means a signal death (guarantee 9) | An exit code from it, by the `128 + n` convention or any other |
| That `exit_code: 0` means the command reported success | That the build is correct — an exit code is the process's claim, not the engine's |
| That the identity is released once the exit arrives (guarantee 15) | That it is released before then — a terminated task keeps its identity until its exit is delivered (FR-019, FR-023) |
| That retained output arriving after an attach is what was missed (guarantee 13) | That the absence of retained output means nothing happened — `retained: 0` also describes a task that has produced nothing yet |

---

## Worked examples

Frames are snake_case and length-prefixed per §4.1; headers omitted. Notifications carry no `id`.

**A chunk of a build's output** (US1, SC-001). `data` decodes to
`   Compiling apex-engine v0.1.0\r\n` with an ANSI green `Compiling`.

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01",
           "data":"ICAgG1sxOzMybUNvbXBpbGluZxtbMG0gYXBleC1lbmdpbmUgdjAuMS4wDQo="}}
```

The escape bytes `0x1B 0x5B 0x31 0x3B 0x33 0x32 0x6D` survive intact (FR-009, SC-002). The engine
did not interpret them and does not know the output is coloured; the panel renders it (FR-027).

**A single unbroken 4 MiB line, chunked** (SC-005, FR-011). No newline anywhere in it. With a
512 KiB chunk bound the delivery is eight frames, in order, with zero truncation.

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01","data":"<699052 base64 characters — 524288 raw bytes>"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01","data":"<699052 base64 characters — 524288 raw bytes>"}}
```

...six more, the last carrying whatever remains. **512 KiB is used here to make the arithmetic
concrete and is not a value the plan has fixed** (task-methods.md, item 8). What the contract fixes
is the ceiling in guarantee 5: a raw chunk cannot exceed `3/4 × (1 MiB − envelope)`, whatever
number the plan chooses below it.

A client concatenates. It MUST NOT treat a chunk as a unit of anything — this line is one line, it
arrived as eight frames, and the eighth is the only one that ends it.

**A task with separate pipes** (`pty: false`, SC-028, FR-008). The two streams are distinguishable
and the process knows it has no terminal, so it emits no colour.

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"lint-03","data":"Y2hlY2tpbmcgNDIgZmlsZXMK"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onStderr",
 "params":{"task_id":"lint-03","data":"d2FybmluZzogdW51c2VkIHZhcmlhYmxlCg=="}}
```

The identical command run with `pty: true` produces **only** `onStdout` frames, with both of those
payloads interleaved into it in the order the process wrote them (guarantee 4).

**An ordinary exit** (FR-020, SC-010). Output first, always (FR-022, SC-011).

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01","data":"ZXJyb3I6IGNvdWxkIG5vdCBjb21waWxlIGBhcGV4LWVuZ2luZWAK"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"build-01","exit_code":101,"signal":null}}
```

The error line is delivered **before** the exit. A client that reported "build failed" from the
exit and then rendered the last chunk would show the failure before its cause; SC-011 asserts zero
lines lost across every exercised case, and ordering is what makes that true rather than lucky.

**An exit by signal** (FR-021, SC-010, SC-012). The developer pressed Ctrl-C in a `pty: true`
panel, the line discipline delivered `SIGINT` to the foreground process group, and the process did
not catch it.

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"build-01","exit_code":null,"signal":"SIGINT"}}
```

`exit_code` is `null` and `signal` names the death. **A client MUST NOT read this as 130.** The
shape above is what guarantee 9 requires; §4.8's row currently makes `exitCode` mandatory and
gives `null` no standing, which is the amendment below.

**Reattaching to a build that ended while nobody was watching** (US5 scenario 3, SC-019, SC-020).
The full sequence, response first.

```json
{"jsonrpc":"2.0","id":"req_task_005","result":{"pid":48300,"running":false,"retained":8192}}
```

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"test-02","data":"<the retained output, chunked>"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"test-02","exit_code":0,"signal":null}}
```

Eight kilobytes owed, eight kilobytes delivered, then the exit. The developer learns the tests
passed while they were disconnected, and the identity is released after this frame (guarantee 15).

---

## Amendments still required

Listed in task-methods.md, *Amendments still required*, and repeated here only where they are
this document's. **None is applied.**

1. **§4.8 must state that `data` is base64** on `onStdout` and `onStderr` (guarantee 1, and
   `writeStdin` in task-methods.md). Describing an existing parameter does not increment
   `protocolVersion`.
2. **§4.8's `onExit` row must become `exitCode?`, `signal?`, exactly one present** (guarantee 9).
   As written it makes `exitCode` mandatory, which leaves a signal death no representation but a
   convention — and spec.md's Key Entities reject a convention in as many words. This is a
   contradiction between the catalogue and the specification, not an omission, and it is the one
   place in this feature where the two disagree about a shape rather than about whether something
   exists.

## What is NOT here

**The methods** — `runTask`, `attach`, `writeStdin`, `resizePty`, `terminate` are task-methods.md.

**The `TaskRunner` port** — runner-port.md. No pseudo-terminal mechanism is named in this
document, and none may be.

**A per-chunk sequence number.** Ordering is guaranteed by the one pipe and the one writer
(guarantee 3, §4.6), so a sequence number would be a field a client could only use to check a
guarantee it already has — and a field that can be checked is a field somebody will build
reordering logic around. If the ordering guarantee ever weakens, the sequence number arrives with
it; not before.

**A drop or gap marker.** FR-013 makes dropping output forbidden rather than rare, so there is
nothing for a marker to mark (guarantee 11). research.md rejected the alternative — drop the
oldest and mark the gap — on the grounds that a build log with a silent hole is worse than a build
that took longer.

**Anything the panel does.** Rendering ANSI, the bounded scrollback (FR-029a), the design-system
palette (FR-030) and staying responsive under load (FR-028) are the client's, and the panel is an
inbound adapter in `client/ui/lib/terminal/`. The engine does not know what a colour is.

**A notification for a task the engine could not start.** FR-004 and SC-015 put that failure in
`runTask`'s response, as a start failure, and in zero `onExit` frames. An engine that answered
`runTask` with a pid and then emitted an immediate `onExit` would satisfy neither.
