# Contract: Task Event Notifications

**Feature**: F010 execution-terminals | **Date**: 2026-09-24

The three `execution/*` notifications this feature implements. All three exist in §4.8 as rows;
the guarantees behind them are written down here, and where this document used to mark a place the
catalogue still had to be amended, the amendment has landed and is cited instead.

Notifications carry no `id` and expect no response (§4.2). These three are engine-originated:
nothing in `dispatch` produces them, nothing correlates them, and every one of them is written
through the single `FrameWriter` F004 introduced —
`engine/src/adapters/outbound/frame_writer.rs`, which holds the output mutex for exactly one
frame. F010 is the first feature to put real volume through that seam, which is what makes
FR-012's obligation a measurement of the seam rather than of this feature alone.

**The three quantities this contract was parametric in are now fixed.** plan.md's *Fixed
Quantities* table sets the chunk size bound at **64 KiB** raw, the chunk time bound at **20 ms**,
and the amount buffered before a process is slowed at **4 MiB** per task (FR-013a). They are used
below as values, not as symbols, and the arithmetic that constrains the first of them is kept
because it is the reason 64 KiB is safe rather than merely chosen.

Rationale for the shapes below is in research.md, *What the `pty` parameter means, and what it
costs*, *Chunking, ordering, and what is pure* and *Backpressure comes for free*. It is not
restated. The decisions are **A-TASKSTREAM**, **A-TASKLIFE** and **A-TASKLIMIT**.

---

## `execution/onStdout` and `execution/onStderr`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `data` | base64 string (§4.8) — **guarantee 1** | yes |

Wire spelling is snake_case: `task_id`, `data`.

The two notifications carry the identical shape and differ only in which stream they name. Which
of them a task ever uses is decided once, by `pty`, at `runTask` (guarantee 4).

---

## `execution/onExit`

| | Name | Type | Required |
|---|---|---|---|
| param | `taskId` | string | yes |
| param | `exitCode` | integer | **exactly when** the task ended with a code — **guarantee 9** |
| param | `signal` | string, signal name — **any** signal the host defines, not only the three `terminate` accepts | **exactly when** the task was killed by a signal |

Wire spelling: `task_id`, `exit_code`, `signal`. **Exactly one of the two is present** (§4.8);
the other key is absent from `params` rather than present and null.

---

## Guarantees

1. **Output is bytes, not text, and `data` is base64** (FR-009, SC-003, §4.8). A process writes
   bytes; a JSON string holds Unicode scalar values, and the two are not the same set. A build
   emitting a filename in a non-UTF-8 locale, a `grep` over a binary, or a progress bar drawn with
   a private-use glyph would all be mangled into replacement characters by a lossy conversion —
   and SC-003 asserts **zero substitutions**. Base64 is the only encoding in the catalogue that
   carries arbitrary bytes.

   §4.8 now states this for `writeStdin`, `onStdout` and `onStderr` together, with the same
   reasoning and one addition worth repeating: `workspace/readFile` carries an explicit `encoding`
   field because a file has two sensible answers, and these payloads have one, so the encoding is
   **fixed here rather than offered**. There is no `encoding` parameter to read and none to send.

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

   §4.8 states this in prose, in the paragraph defining what `pty` chooses.

5. **The chunk size bound is 64 KiB of raw bytes, and it is an order of magnitude under the
   ceiling §4.1 imposes** (FR-011, SC-005, plan.md). The arithmetic is kept because the obvious
   reading is wrong: the bound is **not** 1 MiB, and a bound naively set to 1 MiB overflows the
   frame cap by a third.

   Base64 expands by 4/3. A raw chunk of `N` bytes becomes `4 × ceil(N / 3)` characters, plus the
   JSON-RPC envelope — `jsonrpc`, `method`, `task_id`, the `params` object, quoting — and the
   `Content-Length` header is outside the body while the cap applies to the frame. So the hard
   ceiling on a raw chunk is `3/4 × (1 MiB − envelope)`, around 786 200 bytes for a two-hundred
   byte envelope. 64 KiB encodes to 87 384 characters, roughly a twelfth of the frame budget,
   which leaves the bound immune to an envelope growing by a field and to a `taskId` a client
   chose to make long.

   The value is also a frame-count decision: 64 KiB keeps a 50 MiB burst to roughly 800 frames
   where an 8 KiB chunk would cost 6 400, and every frame is one acquisition of the writer's
   mutex (guarantee 10). SC-005's 4 MiB unbroken line is the case the bound exists for: there is
   no newline to chunk on, so the bound is a byte count and nothing else. Line-based chunking was
   considered and rejected in research.md — a progress bar redrawing with carriage returns emits
   no newline for minutes.

6. **A chunk is emitted on whichever bound is reached first — 64 KiB or 20 ms** (research.md,
   *Chunking, ordering, and what is pure*; plan.md). Size alone starves an interactive process: a
   shell printing a prompt and waiting writes far less than 64 KiB, and the developer would see
   nothing until they typed. Time alone produces frames of unbounded size.

   20 ms is below the threshold at which a prompt reads as delayed and is negligible against the
   round trip to the instance, so it leaves nearly all of SC-001's 500 ms to transport and render.
   Under a burst the size bound dominates and the time bound costs nothing, which is the property
   that lets one pair of numbers serve both an idle shell and a linking build.

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

8. **An exit is reported exactly once to the attachment that has not already received it**
   (FR-020, SC-010, SC-020). There is **one attachment per task** (architecture.md), so "the
   attachment" is the client currently receiving this task's output and the boolean data-model.md
   carries is the whole of the bookkeeping. A client attached when the task ends is told then. A
   client that was away is told when it attaches, after the retained output (task-methods.md,
   `execution/attach` guarantee 3) — and is told **how** it ended in the attach response itself,
   which is a second statement of the same fact and not a second delivery. The notification is not
   re-sent to an attachment that already has it, and re-attaching after collecting it yields
   `-32006` — the identity was released (FR-023).

9. **A signal death is a distinct state, not a conventional exit code** (FR-021, SC-010, §4.8).
   The spec's Key Entities are explicit: an exit is "a code, or a signal. **Distinct states, not
   one field with a convention**". §4.8 now carries the same rule on the wire: `onExit` has
   `exitCode?` and `signal?`, "exactly one of the two and never both". So `signal` is present
   exactly when the task was killed by one, `exit_code` is then **absent**, and a client branches
   on which key it received.

   **The `128 + n` prohibition is kept, and it has moved sides.** The wire can no longer deliver
   a conflated value, so the rule is no longer about what a client may read — it is about what a
   client may **manufacture**. A panel that renders `SIGINT` as "exited 130", or a client store
   that normalises the two fields into one integer before anything else sees them, has rebuilt the
   convention one layer up and thrown away the distinction the protocol just spent a field
   preserving. A process that genuinely exits with code 130 is then indistinguishable from one
   killed by `SIGINT`, which is the exact failure SC-010 measures.

   **The vocabulary here is open, and `execution/terminate`'s is closed. They are different
   sets.** A client may ask for `SIGINT`, `SIGTERM` or `SIGKILL` and nothing else (task-methods.md,
   `terminate` guarantee 5, `-32602` otherwise), because that is where a client *chooses*. What
   *kills* a task is the host's whole vocabulary: `SIGSEGV` from a compiler bug, `SIGPIPE` from a
   closed pager, `SIGHUP`, `SIGKILL` from the out-of-memory killer. FR-020 asks for "the signal
   that killed it" and SC-010 asks for it in 100% of exercised cases, and a segfaulting build is an
   ordinary case. So a client MUST NOT validate this field against the three it may send, and MUST
   NOT fall back to an exit code for a name it does not recognise — an unrecognised signal name is
   still a signal death, and rendering it as a code is guarantee 9's prohibition by another route.

   A frame carrying both keys, or neither, is malformed. There is no state it could describe —
   `Exit` has two variants and no third (runner-port.md) — so a client treats it as a protocol
   error rather than guessing which field to believe.

10. **Output delivery does not delay interactive traffic** (FR-012, §4.6, Principle V, SC-006).
    This guarantee rests on two things, and only one of them existed before F010.

    The one that existed: the reader thread holds the frame writer's mutex for one frame at a time
    and releases it, which is what the writer was built for, and which makes half of this a claim
    about how long the lock is held rather than about how fast the reader is.

    The one F010 builds: **a fairness gate on that same writer**
    (`engine/src/adapters/outbound/frame_writer.rs`). §4.6 now states the rule **per direction**,
    and the engine-to-client half had no implementation — `FrameWriter` is a
    `Mutex<Box<dyn Write + Send>>` and a mutex is first-come by acquisition, so a 50 MiB build
    acquires it roughly 800 times and a completion response takes its turn by arrival. The
    priority-queued behaviour A-PRI implements in `client/core`'s send queue runs client-to-engine
    and does nothing for this direction.

    The gate is a count of writers with interactive traffic waiting. An interactive writer raises
    it, takes the lock, writes and wakes the waiters; a bulk writer yields while the count is
    non-zero, and after **eight consecutive yields** (plan.md's *Fixed Quantities*) goes through
    regardless, so the preference is strong and never a monopoly — F007's language servers are a
    sustained interactive producer and would otherwise stall a build for the length of an index.

    **The engine does not queue, and that is the guarantee, not an implementation detail.**
    Nothing buffers between a reader thread and the wire, so a blocked write still reaches back
    through the pseudo-terminal to the process (guarantee 11, FR-013), and a task's output and its
    `onExit` are written by the one thread in the order it produced them (guarantee 3, FR-022) —
    ordering that comes from the thread rather than from a priority class, which is what a
    two-class queue would have removed. This guarantee therefore depends on a **change to an
    existing component**, and it is unmet until that change exists.

    SC-006 measures it under 50 MiB and **prints the measurement rather than asserting a
    threshold** (A-NFR). This is a measurement obligation, not a comment. A terminal is the
    highest-volume producer the channel will ever carry, and it is the failure mode the whole
    architecture exists to prevent (§1.5, US4) — and the one that would compile, pass every
    functional criterion, and fail only SC-006.

11. **A producer that outruns the link is slowed once 4 MiB is held; nothing is ever dropped**
    (FR-013, FR-013a, SC-021, plan.md). When retained output for a task reaches 4 MiB the reader
    stops reading, the pseudo-terminal's kernel buffer fills, and the process's next write blocks
    — which is what happens to any program writing to a terminal nobody is reading. Nothing else
    is done, and that it needs no mechanism is the point (research.md, *Backpressure comes for
    free*). The bound is per task, so concurrency multiplies the memory held and does not make it
    unbounded.

    The consequence a client must understand: **a gap in time is not a gap in output.** Frames
    stopping for ten seconds means the developer's link is slow, not that anything was lost. There
    is no drop marker, no gap indicator and no truncation notice, because there is nothing to
    mark.

12. **A detached task behaves identically** (FR-031a, SC-021). Output produced while no client is
    attached is retained under the same 4 MiB bound, and a task that outruns it while detached is
    slowed exactly as an attached one would be. A process must not discover it is unobserved by
    being treated differently, and a build that runs faster when nobody is watching is a build
    whose timings mean nothing.

13. **Retained output is replayed in order, before anything produced since** (FR-031b, SC-019,
    §4.8). On attach: the response, then everything retained, then everything new, then — if it
    has ended — the exit. Zero loss and zero reordering. The replay is ordinary
    `onStdout`/`onStderr` frames, chunked by the same two bounds.

    **A client cannot tell a replayed frame from a live one by looking at it, and that is
    correct.** There is no `replayed` flag and no marker frame, because the bytes are the same
    bytes and a panel appends them to the same buffer in the same order either way — a flag would
    exist only to be ignored, or worse, to be branched on by a panel that then renders the two
    differently and shows the developer a seam that is not in their build's output.

    What a client **can** do, and does not need a flag for, is locate the boundary by counting:
    `attach`'s response carries `retained`, so the next `retained` bytes delivered for that task
    are exactly what was missed, and everything after them is live. That is enough for a panel
    that wants to say "you missed 1.2 MB" without inventing a per-frame field, and it is why
    `retained` is a count in the result (task-methods.md, `attach` guarantee 2) rather than a
    property of each frame.

14. **No notification is delivered for a task the client is not attached to.** Starting a task
    attaches the client that started it; `execution/attach` attaches one that did not.
    `execution/list` attaches nothing — a client that enumerates and does not attach receives no
    frames about what it enumerated. A client that has neither started nor attached an identity
    receives nothing about it. With one attachment per task (guarantee 8) "attached" is a state of
    the task and not a set, so this is a rule about which frames exist at all rather than about
    fan-out.

15. **The identity is released after the exit has been delivered** (FR-023, SC-014). Until then it
    is live and cannot be reused (task-methods.md, `runTask` guarantee 2), and it is still listed
    by `execution/list`. After it, `attach` answers `-32006` and `runTask` starts a fresh task.
    SC-014's hundred cycles assert that live identities and running processes both return to their
    starting count, which is the leak this guarantee prevents.

16. **A task terminated by `workspace/close` reports its end like any other** (FR-024, SC-013).
    Closing a workspace ends its tasks by signalling them, so their exits arrive as `onExit`
    carrying `signal`, and their last output precedes those exits under guarantee 7. A client that
    closed the workspace MUST expect the frames: the request it sent is not a suppression of the
    notifications that follow it.

---

## What a client may and may not infer

Stated as a table because every row is a way an implementation has gone wrong before.

| From these notifications, a client MAY infer | A client MUST NOT infer |
|---|---|
| That the bytes in `data` are exactly what the process wrote (guarantee 2) | That they are text, or valid UTF-8, or safe to decode as a string (FR-009, SC-003) |
| That chunk boundaries are where the engine ended a frame | That they are meaningful — a boundary may fall mid-character, mid-escape-sequence or mid-line. A panel must buffer across frames |
| That output for one task is in order (guarantee 3) | Any ordering between two tasks (§4.6) |
| That an `onStdout` frame for a `pty: true` task may hold what the process wrote to either descriptor (guarantee 4) | That `onStderr` will ever arrive for it, or that a silent error stream means no errors (SC-028) |
| That a pause in frames means the link or the 4 MiB bound is holding the process (guarantee 11) | That output was dropped, or that the task is hung. There is no drop, so there is no marker |
| That the first `retained` bytes after an attach are the replay (guarantee 13) | That any individual frame announces itself as replayed — none does, and none should |
| That `onExit` means the task ended and its output is complete (guarantee 7) | That the process's children ended — only `terminate` on the group does that (FR-018, SC-012 for the direct children, SC-027 at arbitrary depth) |
| That `signal` present means a signal death (guarantee 9) | An exit code from it. Neither by reading `128 + n`, which the wire no longer carries, nor by computing one for a panel to show |
| That the name in `signal` is the signal that actually killed the task (guarantee 9) | That it is one of the three `execution/terminate` accepts — `SIGSEGV`, `SIGPIPE`, `SIGHUP` and an out-of-memory `SIGKILL` all arrive here and none can be sent |
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

Thirty-three raw bytes, far under the 64 KiB size bound, so this frame was emitted by the 20 ms
time bound instead (guarantee 6). The escape bytes `0x1B 0x5B 0x31 0x3B 0x33 0x32 0x6D` survive
intact (FR-009, SC-003). The engine did not interpret them and does not know the output is
coloured; the panel renders them, and SC-002 compares the cell grid that results against a
hand-written expected grid (FR-027).

**A single unbroken 4 MiB line, chunked** (SC-005, FR-011). No newline anywhere in it. At the
64 KiB bound the delivery is exactly 64 frames, in order, with zero truncation: 4 MiB is
4 194 304 bytes and 4 194 304 / 65 536 is 64.

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01","data":"<87384 base64 characters — 65536 raw bytes>"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01","data":"<87384 base64 characters — 65536 raw bytes>"}}
```

...sixty-two more, each the same size because the line divides exactly. 87 384 characters is
`4 × ceil(65536 / 3)`, about a twelfth of §4.1's 1 MiB frame, which is the margin guarantee 5
exists to keep.

A client concatenates. It MUST NOT treat a chunk as a unit of anything — this line is one line, it
arrived as sixty-four frames, and the sixty-fourth is the only one that ends it.

**A task with separate pipes** (`pty: false`, SC-028, FR-008). The two streams are distinguishable
and the process knows it has no terminal, so it emits no colour. The payloads decode to
`checking 42 files\n` and `warning: unused variable\n`.

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

**An ordinary exit** (FR-020, SC-010). Output first, always (FR-022, SC-011). The payload decodes
to ``error: could not compile `apex-engine` ``.

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"build-01","data":"ZXJyb3I6IGNvdWxkIG5vdCBjb21waWxlIGBhcGV4LWVuZ2luZWAK"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"build-01","exit_code":101}}
```

There is **no `signal` key**. The error line is delivered **before** the exit: a client that
reported "build failed" from the exit and then rendered the last chunk would show the failure
before its cause; SC-011 asserts zero lines lost across every exercised case, and ordering is what
makes that true rather than lucky.

**An exit by signal** (FR-021, SC-010). The developer pressed Ctrl-C in a `pty: true`
panel, the line discipline delivered `SIGINT` to the foreground process group, and the process did
not catch it.

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"build-01","signal":"SIGINT"}}
```

There is **no `exit_code` key**, not a null one. A client MUST NOT read this as 130, and MUST NOT
write 130 into its own model of the task on the way to the panel (guarantee 9).

**Reattaching to a build that ended while nobody was watching** (US5 scenario 3, SC-019, SC-020).
The full sequence, response first.

```json
{"jsonrpc":"2.0","id":"req_task_006",
 "result":{"pid":48300,"running":false,"retained":8192,"exit_code":0}}
```

```json
{"jsonrpc":"2.0","method":"execution/onStdout",
 "params":{"task_id":"test-02","data":"<the retained 8192 bytes, chunked>"}}
```

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"test-02","exit_code":0}}
```

Eight kilobytes owed, eight kilobytes delivered, then the exit. The response already said how the
task ended, so the panel can show "tests passed" while the transcript is still arriving; the
`onExit` that follows is the same fact in the ordinary place, delivered after the output it
followed (guarantee 8). Everything in the first 8 192 bytes is replay and everything after is
live, countable from `retained` and marked in no other way (guarantee 13). The identity is
released after the exit frame (guarantee 15), and `execution/list` stops listing it.

**A task ended by closing its workspace** (FR-024, SC-013, guarantee 16).

```json
{"jsonrpc":"2.0","method":"execution/onExit",
 "params":{"task_id":"build-01","signal":"SIGTERM"}}
```

`SIGTERM` because that is what the engine sends when it stops a workspace's tasks and no caller
named a signal (plan.md; task-methods.md, `workspace/close` guarantee 2). Had the build ignored it
the frame would have carried `SIGKILL` and arrived five seconds later.

---

## What is NOT here

**The methods** — `runTask`, `attach`, `list`, `writeStdin`, `resizePty`, `terminate` and
`workspace/close` are task-methods.md.

**The `TaskRunner` port** — runner-port.md. No pseudo-terminal mechanism is named in this
document, and none may be.

**A per-chunk sequence number.** Ordering is guaranteed by the one pipe and the one writer
(guarantee 3, §4.6), so a sequence number would be a field a client could only use to check a
guarantee it already has — and a field that can be checked is a field somebody will build
reordering logic around. If the ordering guarantee ever weakens, the sequence number arrives with
it; not before.

**A replay marker.** Guarantee 13 gives the reason and the alternative: the boundary is countable
from `retained`, and a per-frame flag would be a field that exists to be ignored or to be rendered
as a seam the process never produced.

**A drop or gap marker.** FR-013 makes dropping output forbidden rather than rare, so there is
nothing for a marker to mark (guarantee 11). research.md rejected the alternative — drop the
oldest and mark the gap — on the grounds that a build log with a silent hole is worse than a build
that took longer.

**Anything the panel does.** Rendering ANSI, the bounded scrollback — 10 000 lines per terminal,
fixed in plan.md (FR-029a) — the palette (FR-030: the three semantic hues the design system
defines come from tokens, the remaining ANSI colours from the terminal library's own palette as a
recorded exception, A-TERMPALETTE, which is what the narrowed SC-016 measures) and staying
responsive under load (FR-028, measured panel-side by SC-030, which SC-006 does not cover:
a panel can starve while the transport stays healthy) are the client's, and the panel is an
inbound adapter in
`client/ui/lib/terminal/`. The engine does not know what a colour is.

**A notification for a task the engine could not start.** FR-004 and SC-015 put that failure in
`runTask`'s response as `-32011`, and in zero `onExit` frames. An engine that answered `runTask`
with a pid and then emitted an immediate `onExit` would satisfy neither.
