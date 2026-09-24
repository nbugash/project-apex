# Quickstart: Validating Execution Terminals

**Branch**: `feature/F010-execution-terminals` | **Date**: 2026-09-24 | **Plan**: [plan.md](./plan.md)

How to prove this feature works, in the order a reviewer should run it. Every scenario below maps
to a user story or a success criterion, and **none of them needs a remote host or a network**
(FR-033, SC-017).

For what each interface promises, see [contracts/task-methods.md](./contracts/task-methods.md),
[contracts/task-events.md](./contracts/task-events.md) and
[contracts/runner-port.md](./contracts/runner-port.md). For the task, chunk, exit and panel
entities and what persists across a client restart, see [data-model.md](./data-model.md). This
document restates neither — it says how to run them and what must be true afterwards.

**On the commands below.** Every `make` target and every `npm run` script named here exists today
— check `Makefile` and `package.json`. Every `cargo test --test <name>` and every spec file under
`tests/` names a target **this feature adds**; none of them exists on `master`, and `tasks.md` is
where they are created. If a target below does not exist when you run it, the task that creates it
has not been done. That is a finding, not a typo to work around.

**On the one thing this feature cannot prove.** A-TASKLIFE makes a *task* outlive the connection
that started it. Whether the *engine* outlives the client is F020 `detached-engine`'s, and the
specification says so. Everything below models a disconnection as **the transport closing while
the engine process stays up**, because that is the boundary F010 owns. A test that kills the
engine and expects a task to survive is testing F020, and it will fail for the right reason.

---

## Prerequisites

```bash
cargo --version            # 1.75 or newer — the MSRV all three crates declare
node --version             # 20 or newer
make setup                 # npm ci
uname -s                   # Linux, for every engine-side section below
make next F=F010           # where this feature stands in the pipeline
```

Three environment facts decide what you can run:

- **The pseudo-terminal adapter is Linux-only by construction.** The mechanism is named in exactly
  one file (plan.md, *Structure Decision*), guarded by `engine/tests/pty_confinement.rs`. On macOS
  and Windows the domain, application and client suites all run; the engine-side sections are the
  ones that do not, and local mode's task methods return `Unsupported` by decision (research.md,
  *Local mode is F015's*, and §13.2 as amended). That is a stated degradation with its own test,
  not a hole.
- **`make test` builds the engine binary first** — `cargo build -p apex-engine` is its opening
  line. This matters more here than it did for F004: several sections spawn that built binary as a
  local child process and then spawn *further* processes under it, so a stale
  `target/debug/apex-engine` fails them for a reason that has nothing to do with terminals.
- **Several checks count processes and read `/proc`.** They assume the test runner is the same
  user as the spawned tasks, which A-EC2's single tenancy guarantees on the instance and your
  shell guarantees here. Under a container that hides `/proc`, the process-count checks
  (SC-012, SC-013, SC-014, SC-027) degrade to counting what the engine believes it holds, which is
  the weaker half and says so.

Read the host's own limits first, so a failure in the resource-limit section is legible rather
than mysterious:

```bash
ulimit -a
cat /proc/sys/kernel/pid_max
```

---

## 0. What stands in for the remote host

Nothing below talks to a host. Each scenario says which double it uses, because a scenario whose
stand-in is unstated is a scenario nobody can reproduce. Four doubles and one real process, and
the split between them is the whole design: everything that **decides** something is tested
without a process, and the real process is reserved for the questions only a kernel can answer.

| Stand-in | Replaces | Lives in | What it must not be asked to do |
|---|---|---|---|
| `FakeTaskRunner` | The pseudo-terminal and the process | `engine/tests/common/` | Decide anything. It is handed bytes to emit and an exit to report, so a 50 MiB burst is a loop, not a build |
| `FakeClock` (engine `Clock` port) | Elapsed time | `engine/tests/common/fake_clock.rs`, already there for F004 | Be replaced by `sleep`. The chunker's time bound is a value it is told, not a wait |
| `apex-mock-daemon` | The remote `sshd` and its link | `client/core/tests/mock_daemon/` | Understand a request body. It answers framing and timing only — `delay=250,drop=20`, `stall=<ms>`, `close-mid-frame`, and `notify=<ms>` for a server-originated frame |
| Recording fake transport | The wire, when the question is "how many" | `client/core/tests/common/` | Be substituted by a log grep. A log-shape change silently passes a grep and silently fails nothing |
| The locally spawned real engine | The remote engine process | `target/debug/apex-engine`, spawned on real pipes through `ProcessSpawner` | Reach a network. It is a child process on this machine; the transport under test is the production one |
| A real locally spawned process under a real pty | The developer's build | Fixture programs created by `tasks.md`, spawned by the engine above | Be faked for `isatty`, `SIGWINCH`, process groups or resource limits — a fake cannot prove any of the four |
| `npm run stub:connection -- --state disconnected` | A dropped link, for the running shell | `scripts/stub-connection.mjs` | Appear in an automated assertion. It drives a manual look, not a gate |

**Which fixture programs the real-process sections need**, named here so `tasks.md` creates them
rather than each test inventing one:

- one that reports whether its output is a terminal, and writes a known string to each stream;
- one that reports its window size whenever it changes, for the resize measurement;
- one that reads its input and echoes it back byte-for-byte, including control bytes;
- one that reports the signal it caught rather than dying quietly from it;
- one that spawns a child which spawns a grandchild, all three of which outlive the parent unless
  something stops the group;
- one that writes a 4 MiB line with no newline in it, and one that writes 50 MiB as fast as it can;
- one that writes a known non-UTF-8 byte sequence;
- one that allocates until it is stopped.

**The gap in this row of doubles, stated before you hit it.** F004 added `notify=<ms>` to the mock
daemon and F010 is its second user — but it emits **one** frame, supplied whole in
`APEX_MOCK_FRAME`, at one delay. A terminal's traffic is a hundred ordered frames, and the mock
cannot produce them without either a second directive or a body it understands, which its own
`the_mock_implements_no_engine_method` guard exists to prevent. So the mock proves that a
server-originated output frame survives latency and loss; it does not prove ordering, volume or
reattachment replay, and the locally spawned engine is the only route to those. See *Known gaps*.

---

## 1. The whole suite

```bash
make test
```

That is `cargo build -p apex-engine`, `cargo test --workspace`, clippy with `-D warnings`,
`cargo fmt --all --check`, `npm run test:unit`, `npm run lint`, `npm run lint:ds` and
`scripts/pipeline_test.py` — the gate as the Makefile defines it, so nobody reconstructs it by
hand and gets it subtly different.

**Read the summary line, not the exit code.** F001 found a gate reporting `failed=no` while six
tests failed, because `test result: FAILED.` puts the word in the third field and the check looked
at the second. If a number other than zero appears after `failed:`, the run failed.

---

## 2. US1 — run a command and watch it work

**Set up**: a workspace registered against a real temp directory (no host). The `FakeTaskRunner`
for everything the chunker decides, `FakeClock` for the time bound, and a locally spawned real
process under a real pty for the three questions a fake cannot answer — `isatty`, merged streams,
and colour surviving the journey.

```bash
cargo test -p apex-engine --lib output
cargo test -p apex-engine --test task_streams
cargo test -p apex-engine --test task_chunking -- --nocapture
cargo test -p apex-engine --test task_containment
cargo test -p apex-shell  --test observe_task
npm run test:unit -- tests/unit/terminal-render.test.ts
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US1.1 | Run a fixture writing a line every 50 ms for 5 s | Chunks arrive throughout, not one at the end; the first arrives before the process exits — measured, see §8 |
| US1.2 | Run a fixture writing to both streams with `pty: true` | Both appear on `execution/onStdout`, interleaved; `execution/onStderr` byte count is **0** (A-TASKSTREAM) |
| US1.2a | The same fixture with `pty: false` | The two arrive on separate notifications and are separable; the fixture reports `isatty` false |
| US1.3 | Run a fixture emitting ANSI colour and cursor movement | The panel's cell grid carries the attributes; the escape bytes appear in zero rendered cells |
| US1.4 | Start a task with `cwd` set to a subdirectory | The process's own working directory is that subdirectory, read from the process rather than from the request |
| US1.4a | Start a task with `cwd` escaping the workspace root, in each escape shape | Refused by the engine, independently of the client (FR-003, §4.7) |
| US1.5 | Start a task with an environment variable supplied | The process sees the supplied value; it also sees the engine's `PATH`, because the environment is inherited and overridden rather than replaced |

US1.2 and US1.2a are one scenario split in half, and the half that matters is the **zero**. A-TASKSTREAM
says a terminal is one device; an implementation that opens a pty and then quietly keeps a
separate pipe for errors passes every positive assertion here and fails only the byte count.

US1.4 reads the working directory **from the process**, not from the engine's record of it. The
record is what the engine intended; the process is what happened.

---

## 3. US2 — type back, and mean it

**Set up**: a real locally spawned process under a real pty for all of it. Input reaching a
process, an interrupt arriving as a signal, and a window size changing are three things no
in-memory double can prove, because the proof is what the kernel does.

```bash
cargo test -p apex-engine --test task_input
cargo test -p apex-engine --test task_signals
cargo test -p apex-engine --test task_resize -- --nocapture
cargo test -p apex-engine --test task_process_group
npm run e2e                        # tests/e2e/terminal.spec.ts, Linux, per A-E2E
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US2.1 | Write a string containing control bytes and a non-UTF-8 sequence to a task's input | The echo fixture returns the identical bytes; compared as bytes, not as a string |
| US2.2 | Send an interrupt to a running task | The signal fixture reports the signal it caught; its recorded input contains **zero** `0x03` bytes |
| US2.3 | Resize the panel, 100 times, to alternating dimensions | The window-size fixture reports each new size; latency measured and printed, see §8 |
| US2.4 | Run a fixture that asks whether it is attached to a terminal | It reports yes, and it colours its output without being asked to |
| US2.5 | Ask to stop a task, and ask again after it has gone | Both requests succeed (FR-019); the second is not an error for racing an exit |
| US2.6 | Stop a task whose fixture ignores the first signal | It is still stopped; the escalation is the one the plan names, not an escalation invented by the test |

US2.6 has a dependency the plan has not discharged: **which** signal is sent first, how long the
escalation waits, and which signal follows are plan-level values by the specification's own
Assumptions, and plan.md states none of them. Until it does, this scenario can assert that the
process ends and cannot assert that it ended the way somebody chose. See *Known gaps*.

---

## 4. US3 — know how it ended

**Set up**: `FakeTaskRunner` for the reporting shape and the identity lifecycle, real processes
for the process-tree and workspace-close counts, because "zero running" is a claim about the
operating system.

```bash
cargo test -p apex-engine --test task_lifecycle
cargo test -p apex-engine --test task_start_failure
cargo test -p apex-engine --test task_ordering
cargo test -p apex-engine --test task_workspace_close
cargo test -p apex-engine --test task_leak
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US3.1 | Run a command that exits 7 | The exit is reported carrying 7, not carrying "non-zero" |
| US3.2 | Kill a task's process with a signal | Reported as a signal death, in a **different state** from an exit code — not one field with a convention (FR-021, Key Entities) |
| US3.3 | Let a task exit, then start a new one under the same identity | The old identity is released after its last chunk is delivered, and the reuse succeeds |
| US3.3a | Try to reuse an identity that is still live | Refused (SC-022) — the same check US5.4 makes from the other side |
| US3.4 | Close a workspace holding three running tasks | Zero of its processes survive, counted in `/proc`, including children |
| US3.5 | Run a command that does not exist | The `runTask` request fails; **zero** `execution/onExit` notifications carry that id |
| US3.6 | Write output and exit in the same breath, 100 times | The last chunk precedes the exit at the outbound sink in all 100 |

US3.5 is the edge case the specification opens with, and the assertion that matters is the zero.
An implementation that reports a start failure *and* an exit is telling the client two things
about one event, and the client that believes the second one thinks a build ran.

US3.6 needs the fixture to write **immediately before exiting, without flushing and waiting**. A
fixture that sleeps after its last write gives the delivery path all the time it needs, and the
scenario passes for an implementation that reorders.

---

## 5. US4 — a build must not freeze the editor

**Set up**: `FakeTaskRunner` driving 50 MiB through the real chunker and the real `FrameWriter`
F004 built, `FakeClock` for the time bound, and the recording transport counting and timing what
crosses the boundary. No process: the question is what the queue does, and §4.6 is about the
queue.

```bash
cargo test -p apex-engine --test task_budget    -- --nocapture
cargo test -p apex-engine --test task_retention -- --nocapture
npm run perf:budget                # tests/perf/terminal-budget.spec.ts
npm run e2e                        # tests/e2e/terminal-responsive.spec.ts
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US4.1 | Issue interactive requests throughout a 50 MiB burst | **Printed** p99 against §1.4's 250 ms, ≥100 samples, see §8 |
| US4.2 | Let a producer outrun the link until retention is full | The reader stops reading; bytes held stop growing; bytes delivered eventually equal bytes written (FR-013) |
| US4.2a | The same, with no client attached | Identical behaviour and identical bound (FR-031a). The process must not discover it is unobserved by being treated differently |
| US4.3 | Drive output at the panel faster than it renders | The panel answers input throughout; retained history stays within its bound |

US4.1 is the measurement this whole architecture exists to pass, and F010 is the first feature to
put real volume through F004's frame writer. If it fails, the finding is about the seam, not about
terminals.

US4.2's assertion must be on **bytes held**, and §9 says why: an implementation that buffers
everything also loses nothing.

---

## 6. US5 — come back to a build that kept going

**Set up**: the locally spawned real engine holding a real task, with the client's transport
closed under it — that is the disconnection F010 owns. The mock daemon's `stall=<ms>` and
`close-mid-frame` for the client half, and `notify=<ms>` with `APEX_MOCK_FRAME` for a
server-originated frame arriving under loss.

```bash
cargo test -p apex-engine --test task_detach
cargo test -p apex-engine --test task_reattach
cargo test -p apex-shell  --test task_reconnect
cargo test -p apex-shell  --test task_restart
cargo test -p apex-shell  --test task_summary
```

| Scenario | What to do | What must be observed |
|---|---|---|
| US5.1 | Close the transport under a running task | The process is still alive afterwards; zero exits attributable to the close |
| US5.2 | Write output while detached, then reattach | Every byte written while away is delivered, in order, **before** anything written since — checked as a sequence, not as a set |
| US5.3 | Let a task exit while detached, then reattach | The attach reports it is no longer running, and how it ended (SC-020) |
| US5.4 | Start a task under an identity that is already running | Refused; the process count under that identity is unchanged |
| US5.5 | Restart the client, reload its stored state, reattach | Every identity the store holds attaches; the developer is told which survived and what was missed (FR-032) |
| US5.6 | Reattach to an identity that was never started | Refused, distinguishably from an identity that ran and ended |

US5.2 is the scenario that distinguishes reattachment from resumption. Checking that the missed
bytes are *present* passes for an implementation that appends them after the live stream, which is
a transcript in the wrong order — and a developer reading a build log has no way to notice.

US5.5 is where `execution/attach` earns the amendment. Read §4.8 as amended before reviewing it:
attaching is a different call from starting **on purpose** (research.md, *Attaching to a task that
is already running*), and the test that proves it is the one in §9 that counts processes.

---

## 7. Every success criterion

All 28. Each row names the command and the **observable** — the thing you look at, not the thing
the code intends.

| SC | Claim | Command | Observable |
|---|---|---|---|
| SC-001 | Output in the panel within 500 ms | `cargo test -p apex-engine --test task_latency -- --nocapture` | **Printed** p99 in ms over ≥100 writes, harness delay excluded (§8) |
| SC-002 | ANSI renders as it does in a local terminal, 100% of exercised cases | `npm run e2e` (`terminal-ansi.spec.ts`), `npm run gate:fidelity` | Rendered cell grid after each scripted sequence equals the expected grid. **Partial** — no local-terminal oracle exists; see *Known gaps* |
| SC-003 | Byte-for-byte, non-UTF-8 included, zero substitutions | `cargo test -p apex-engine --test task_binary_output`, `cargo test -p apex-shell --test observe_task` | Digest of what the fixture wrote equals the digest of what arrives at the client's inbound adapter; zero U+FFFD anywhere in the delivered bytes |
| SC-004 | Order preserved, 100% of exercised cases | `cargo test -p apex-engine --lib output`, `cargo test -p apex-engine --test task_ordering` | A monotonically numbered fixture stream reassembles with zero gaps and zero transpositions |
| SC-005 | A 4 MiB line delivered whole, each chunk within the frame cap | `cargo test -p apex-engine --test task_chunking -- --nocapture` | **Printed** total bytes and largest chunk: total exactly 4 MiB, largest < 1 MiB (§4.1), zero truncation |
| SC-006 | Interactive budget holds through 50 MiB | `cargo test -p apex-engine --test task_budget -- --nocapture` | **Printed** p99 for interactive requests during the burst, against §1.4's 250 ms (§8) |
| SC-007 | Typed input reaches the process unmodified | `cargo test -p apex-engine --test task_input` | Echo fixture returns identical bytes, compared as bytes; includes control bytes and a non-UTF-8 sequence |
| SC-008 | Interrupt arrives as a signal in 100%, as text in zero | `cargo test -p apex-engine --test task_signals` | Signal fixture reports the signal caught; its recorded input contains zero `0x03` bytes |
| SC-009 | Resize observed within 500 ms | `cargo test -p apex-engine --test task_resize -- --nocapture` | **Printed** p99 over ≥100 resizes, from the notification leaving the boundary to the fixture reporting the new size (§8) |
| SC-010 | Exit code reported 100%; signal death distinguishable 100% | `cargo test -p apex-engine --test task_lifecycle` | Two distinct reported states across both fixtures, not one field carrying a convention |
| SC-011 | Output before an exit delivered before the exit, zero lost | `cargo test -p apex-engine --test task_ordering` | Index of the final chunk < index of the exit at the outbound sink, across 100 runs; delivered byte count equals written |
| SC-012 | Terminating leaves zero processes, children included | `cargo test -p apex-engine --test task_process_group` | `/proc` scan of the task's process group after termination: zero survivors |
| SC-013 | Closing a workspace leaves zero tasks running | `cargo test -p apex-engine --test task_workspace_close` | The same scan, for every task of that workspace, after `workspace/close` |
| SC-014 | 100 cycles return identity and process counts to start | `cargo test -p apex-engine --test task_leak` | Live-identity count and child-process count read before and after the loop, both identical |
| SC-015 | An unstartable command is a start failure 100%, a task exit in zero | `cargo test -p apex-engine --test task_start_failure` | Error on the `runTask` response; **zero** `execution/onExit` frames carrying that id |
| SC-016 | Panel colours resolve to design-system tokens, zero raw values | `npm run lint:ds`, `npm run test:unit -- tests/unit/terminal-palette.test.ts`, `npm run e2e` (`terminal-tokens.spec.ts`) | Source: lint passes. Rendered: computed colour of each of the 16 ANSI slots equals a `--color-*` token value. **Partial** — see *Known gaps* |
| SC-017 | The suite runs with no remote host and no network | `make no-network` | The Rust suite passes with no interfaces; the target states its own degraded fallback where user namespaces are unavailable |
| SC-018 | A task survives a drop 100%, zero terminated by the drop alone | `cargo test -p apex-engine --test task_detach` | Process alive after the transport closes; exits attributable to the close counted, and the count is 0 |
| SC-019 | Reattach receives every missed byte, in order, first | `cargo test -p apex-engine --test task_reattach` | Concatenated retained bytes equal what was written while detached; the first post-attach chunk's index is the last pre-drop index plus one |
| SC-020 | An exit while detached is reported on reattach, 100% | `cargo test -p apex-engine --test task_reattach` | The attach result reports not running and carries how it ended; the exit is then delivered |
| SC-021 | A detached overrun is slowed, zero bytes dropped | `cargo test -p apex-engine --test task_retention -- --nocapture` | **Printed** bytes held and bytes delivered: delivered equals written, held stays at or under the bound (§8) |
| SC-022 | Starting under a live identity refused 100%, second process in zero | `cargo test -p apex-engine --test task_reattach` | Error on the second `runTask`; the process count under that identity is unchanged |
| SC-023 | A restarted client reaches every task it started, zero unreachable | `cargo test -p apex-shell --test task_restart`, `cargo test -p apex-shell --test session_migration` | Every identity in the reloaded store attaches successfully. **Partial** — the "zero unreachable" half; see *Known gaps* |
| SC-024 | Retained history within its bound across 50 MiB | `npm run test:unit -- tests/unit/terminal-history.test.ts`, `npm run perf:budget` | **Printed** bytes retained by the panel after the burst, against the bound the source exports. **No bound is stated in plan.md today** — see *Known gaps* |
| SC-025 | Environment in zero log lines and zero crash reports | `cargo test -p apex-engine --test task_env_redaction` | A sentinel value appears in no captured log line and no crash payload, including on the start-failure path |
| SC-026 | A process over its memory limit dies within 2 s; engine survives 100% | `cargo test -p apex-engine --test task_limits -- --nocapture` | **Printed** interval from the crossing allocation to the exit notification; the engine answers a subsequent request. **No limit is stated in plan.md today** — see *Known gaps* |
| SC-027 | Stopping leaves zero spawned processes, at any depth | `cargo test -p apex-engine --test task_process_group` | `/proc` scan across three generations of the fixture's tree: zero survivors at every depth |
| SC-028 | With a pty: `isatty` true, zero stderr bytes; without: false and separated | `cargo test -p apex-engine --test task_streams` | Fixture prints its own `isatty` verdict; `onStderr` byte count is 0 in the pty case, non-zero and separable in the other |

---

## 8. The measurements (A-NFR binds all six)

Six criteria are performance or volume claims: **SC-001** (500 ms to the panel), **SC-005** (a
4 MiB line), **SC-006** (the budget under 50 MiB), **SC-009** (500 ms to observe a resize),
**SC-021**/**SC-024** (bytes held) and **SC-026** (2 seconds to die). For **each** of them A-NFR
applies in full, and this guide states it once so no measurement quietly drops a clause:

- **p99**, not mean and not max.
- Measured at the **interface/transport boundary**, not wall clock end to end.
- **At least 100 samples.**
- **Any delay the harness itself injects is excluded** — the mock daemon's 250 ms in particular,
  and the fixture process's own scheduling where the fixture timestamps its own writes.
- **The measured value is printed, not merely compared.** A gate that says only PASS tells nobody
  how much headroom is left, which is what says whether the next feature can be afforded.

```bash
cargo test -p apex-engine --test task_latency   -- --nocapture
cargo test -p apex-engine --test task_budget    -- --nocapture
cargo test -p apex-engine --test task_resize    -- --nocapture
cargo test -p apex-engine --test task_chunking  -- --nocapture
cargo test -p apex-engine --test task_retention -- --nocapture
cargo test -p apex-engine --test task_limits    -- --nocapture
npm run perf:budget
```

Expected shape of the output, with the numbers to be filled by the run:

```
SC-001  write to chunk delivered      p99 = ___ ms    budget  500 ms   (n=___)
SC-005  largest chunk, 4 MiB line     max = ___ B     cap 1048576 B    (total=___ B)
SC-006  interactive req under burst   p99 = ___ ms    budget  250 ms   (n=___, burst=50 MiB)
SC-009  resize to observed size       p99 = ___ ms    budget  500 ms   (n=___)
SC-021  bytes held / bytes delivered  held = ___ B    bound ___ B      (written=___ B)
SC-024  panel history after 50 MiB    held = ___ B    bound ___ B
SC-026  over-limit to exit reported   p99 = ___ ms    budget 2000 ms   (n=___)
```

**SC-001's measurement point has to be stated, because it is the one that can be measured wrongly
and look right.** The claim is 500 ms from the process writing to the output appearing in the
panel, and that spans two processes and a renderer. Measure it as **one** interval across the
locally spawned engine — the fixture timestamps its own write, the clock stops when the chunk
crosses the client's inbound boundary — because summing an engine-side p99 and a client-side p99
is not a p99 of the whole: two independent 99th percentiles composed give a 98th percentile bound,
and reporting that as p99 overstates what was measured. If the single-interval measurement is not
achievable, report the composition **as a p98 bound** and say so. This is the same trap F004
recorded for its SC-001, and it is worse here because there are more hops to be tempted by.

**Every bound must be derived from the source, not typed into the test.** The chunker's size
bound, the chunker's time bound, the retention bound and the panel's history bound are all values
somebody chooses (FR-006b, FR-013a, FR-029a); the test must read them from the constant the source
exports and compute against that. A test carrying its own `65536` passes after somebody changes
the chunk size, which is the moment the assertion stopped meaning anything — F004's SC-007 learned
this and it applies four times over here.

**And the values do not exist yet.** plan.md fixes 500 ms, 500 ms and 50 MiB, all of which are
restatements of success criteria; it fixes none of the four quantities the requirements above
demand of it. Four of the seven rows in the block above have no number in their `bound` column
today. That is the first entry in *Known gaps* and it blocks implementation, not just measurement.

---

## 9. The negative checks

Each of these is a claim that something does **not** happen. A negative check that cannot fail is
worthless, so each row says what would make it fail — if that condition is not in the fixture, the
check is decoration.

| Must not happen | How you would see it | What makes the check able to fail |
|---|---|---|
| **A byte on the error stream when a terminal is attached** (SC-028) | `onStderr` byte count for the task, counted at the engine's outbound sink before serialisation | The fixture must **write to standard error**, loudly, and the same fixture must be run with `pty: false` in the same suite. With a fixture that only writes to standard output the zero is free, and an implementation that keeps a second pipe open passes |
| **Output lost when a producer outruns the link** (SC-021) | Written byte count at the fixture compared with delivered byte count at the sink, **and** bytes held at the retention point | The producer must outrun the consumer for long enough to fill the bound — a fixture that fits inside it never triggers the mechanism. The held-bytes half is what stops an unbounded buffer passing: it loses nothing either |
| **A process surviving a stop, at any depth** (SC-027) | `/proc` scan of the process group after termination, walked to three generations | The fixture must spawn a **grandchild**, and both descendants must ignore the signal their parent gets. A two-level fixture passes for an implementation that signals the direct child only, which is exactly the bug FR-006a exists to prevent |
| **A task terminated by a disconnection alone** (SC-018) | The task's process still present after the transport closes, plus a count of exits attributable to the close | The engine must still be running when the transport closes — if the harness kills the engine, everything dies and the check fails for F020's reason instead of F010's. And there must be a task with **no** reason of its own to exit during the window |
| **A second process under a live identity** (SC-022) | Process count under the identity's group before and after the refused `runTask` | The first task must still be **running** at the moment of the second call. Against a task that has exited, reuse is legitimate (FR-023) and the refusal would be the bug. Assert on the process count, not on the error: an implementation that spawns and then reports an error passes an error-only assertion |
| **A task's environment in a log line** (SC-025) | A sentinel value grepped across every captured log line and crash payload for the run | The environment must contain a value that could only have come from the environment, and the task must be exercised on **both** paths — started successfully, and failed to start. The failure path is where the request is most likely to be logged whole |

---

## 10. Mutation checks

This project's practice: break the thing deliberately, confirm the test fails, revert. Per A-TEST,
a check guarding a property that would otherwise be invisible is verified by breaking the property.
Six worth running here, the first four at minimum.

| Mutation | Test that must fail | What it is really testing |
|---|---|---|
| **Emit only on the size bound** — delete the chunker's time bound, so a partial chunk waits for more bytes | `task_latency`'s SC-001 measurement, and the `--lib output` case for a sub-threshold write | Whether the suite has an **interactive** case at all. Every volume test still passes — a 50 MiB burst fills the size bound constantly — while a shell printing a prompt and waiting emits nothing and the developer sees an empty panel. If nothing fails, that absence is the finding |
| **Remove the process group** — spawn the task without one, so a stop signals the named process only | `task_process_group`'s SC-027 assertion | Whether the fixture has a grandchild. With a one-level fixture the mutation is invisible, and FR-006a is untested for exactly the case it was written for |
| **Let the reader keep reading past the retention bound** | `task_retention`'s SC-021 **held-bytes** assertion | Whether the assertion is on memory held or merely on output arriving. Nothing is dropped by an unbounded buffer, so a delivered-bytes-only check passes this mutation happily, and the failure it was guarding is a build's output on the instance's heap |
| **Deliver the exit before the last output chunk** — report the exit as soon as it is observed, without draining | `task_ordering`'s SC-011 assertion | Whether the fixture writes immediately before exiting. Any fixture that flushes and pauses hands the delivery path enough slack to reorder undetectably, and the last lines of a failing build are precisely what this criterion exists to keep |
| **Make `runTask` attach when the identity is live** — the alternative research.md rejected | `task_reattach`'s SC-022 assertion | Whether the assertion counts **processes** or reads the response. This mutation returns a perfectly plausible success, so a test that trusts the envelope reports a clean pass over two builds running under one identity |
| **Encode output as a lossy UTF-8 string** on the way to the wire | `task_binary_output`'s SC-003 assertion | Whether the fixture writes a byte sequence that is not valid UTF-8. With ASCII-only fixtures every substitution is a no-op and the criterion is vacuous |

Run each as: apply the mutation, run the named target, confirm it fails with the assertion you
expected rather than a compile error, revert, confirm green.

---

## 11. The gates before merge

```bash
make gate                  # make test, plus npm run e2e, npm run gate:fidelity, and make no-network
make verify F=F010
```

Three design-system obligations are not covered by `lint:ds`, which restricts raw hex, raw pixels
and font families in `client/ui` and has no view of any of them. F010 adds a whole new surface, so
all three apply:

- **The terminal library's default palette is a raw value like any other** (Principle I,
  plan.md's Constitution Check). It lives in a dependency, where `lint:ds` cannot see it, so the
  assertion has to be on **computed style** in a browser — following `token-conformance.spec.ts`.
- **A channel other than colour** for the panel's ended state (FR-029), asserted in greyscale,
  following `rail-greyscale.spec.ts` and F003's `cache-verification.spec.ts`.
- **Keyboard reachability** for anything the panel makes actionable, following
  `rail-keyboard.spec.ts`.

The prototype is unusually specific here — a default terminal dock, JetBrains Mono at 12.5px, a
7x15px block cursor, a title that switches on mode — and **12.5px is a third font size** that
`ds-sync` must extract rather than a component inventing it (plan.md, Principle I). If
`mockups/Apex IDE (standalone).html` has no state for something this feature needs, that is a
deviation needing written designer approval recorded **before** implementation — not an invented
style, and not a fidelity baseline quietly refreshed with `gate:fidelity:update`.

If the browser-driven gates need a display, F003's quickstart records that `xvfb-run` wrapping the
command was not enough and a persistent `Xvfb :77` was; `make gate` already defaults `DISPLAY` to
`:77` for exactly that reason.

---

## 12. Validation record

*To be completed when the feature is implemented. Left empty deliberately: a table of results
nobody produced is the failure mode A-TEST names — confidence that has not been earned.*

| Check | Result |
|---|---|
| `make test` | |
| `make gate` | |
| `make no-network` (SC-017) | |
| The seven printed measurements (§8) | |
| The six negative checks (§9), each with its fixture condition confirmed present | |
| The six mutations (§10), each failing then reverted | |

---

## Known gaps

Stated here rather than discovered by a reviewer.

1. **The plan states none of the four quantities the requirements demand of it, and this blocks
   four criteria.** FR-006b requires the resource limits to be "stated quantities fixed in the
   plan". FR-013a requires the same of the amount buffered before a process is slowed. FR-029a
   requires the same of the panel's retained history. research.md's *Chunking, ordering, and what
   is pure* adds two more, saying of the chunker's size and time bounds that "both are values
   fixed in the plan". **plan.md fixes none of them.** Its Performance Goals restate SC-001,
   SC-009 and SC-006 and stop there. The consequence is concrete: SC-024 and SC-026 have no
   threshold to compare a printed number against, SC-021's bound half is unprovable, SC-005's
   chunk-size expectation has to be inferred from §4.1's cap rather than from the chosen value,
   and §3's escalation scenario cannot assert which signal was sent. The measurement harness is
   runnable today; **the thresholds are not, because nobody has chosen them.** This is a
   plan-level omission to close before implementation, not a test to write around.

2. **SC-002 has no oracle.** The criterion says a command "renders identically to the same command
   in a local terminal". Nothing in this repository is a local terminal, and comparing the panel
   against the same library rendering headlessly compares it with itself. What is runnable is a
   scripted sequence with an expected cell grid written by hand, plus the fidelity gate against
   the prototype — which proves the panel matches what was specified, not that it matches what
   `xterm` on the developer's machine does. Closing it properly needs a reference terminal in the
   fixture set, which is a dependency this feature has not taken. Verified as stated: **partial**.

3. **The mock daemon still cannot carry a stream.** F004 added `notify=<ms>`, which emits one
   caller-supplied frame from `APEX_MOCK_FRAME`, and F010 is its second user — as intended. But a
   terminal's traffic is a hundred ordered frames, and one frame at one delay cannot exercise
   ordering (SC-004), replay on reattach (SC-019) or volume (SC-006). Extending the directive to a
   sequence is possible within the mock's own rule, since the frames stay opaque to it; until then
   the transport-facing half of those criteria runs only against the locally spawned engine, and
   the loss-and-latency path they would otherwise cover is untested for output specifically.

4. **There is no way to ask the engine what tasks it is holding, so SC-023's second half is
   unprovable.** §4.8 as amended has `execution/attach`, which takes a `taskId` the client must
   already know. A client whose stored identities survive its restart is fully testable and the
   command above tests it. A client that loses them has no route back — and SC-023 asserts "zero
   running tasks unreachable", which is a claim about exactly that case. The specification accepts
   the hole and bounds it with A-EC2's thirty-minute idle stop, which is F005's, not this
   feature's. This is the seventh absence of this kind in the catalogue; whether it becomes an
   `execution/list` is a decision, not an oversight, and it should be recorded as one rather than
   left as a criterion nobody can run.

5. **SC-003 depends on an encoding that `contracts/` has not published yet.** §4.8 carries output
   as `data` on a JSON-RPC notification, and a JSON string cannot hold arbitrary bytes. Some
   encoding is therefore required for non-UTF-8 output to survive, and it is a contract decision
   being made in parallel with this document. The check above is written so that it is the thing
   that catches a wrong answer — digests compared at both ends, zero U+FFFD — but it cannot be
   run until the contract says what the field holds.

6. **SC-026 needs a real kernel, a real process, and a measurable moment of crossing.** No double
   can prove a resource limit fires, so this criterion is Linux-only and needs the allocating
   fixture. The 2-second interval is measured from the allocation that crosses the limit to the
   exit notification, and the fixture has to report the moment it crossed — the engine cannot
   observe it. Where the environment refuses to apply the limit at all, the criterion is skipped
   rather than passed, and a skipped criterion must appear in the validation record as a skip.

7. **The process-count criteria degrade where `/proc` is restricted.** SC-012, SC-013, SC-014 and
   SC-027 are all "zero running" claims, and the only honest observable is the operating system's.
   In a container that hides other processes, they fall back to counting what the engine believes
   it holds — which passes for an implementation whose bookkeeping is right and whose signalling
   is not, i.e. for the bug. Run them on the instance, or on a host where `/proc` is whole.

8. **SC-017's network check depends on the host, and must not be read too strictly.**
   `make no-network` uses `unshare -rn` where unprivileged user namespaces are available and
   degrades to a source scan where they are not, which is weaker and says so. Note that F010's
   suite deliberately spawns real local processes: that is not a network dependency, and a future
   tightening of this check that forbids spawning would fail this feature for the wrong reason.

---

## What "done" looks like

```bash
make test \
  && cargo test -p apex-engine --test task_latency   -- --nocapture \
  && cargo test -p apex-engine --test task_budget    -- --nocapture \
  && cargo test -p apex-engine --test task_resize    -- --nocapture \
  && cargo test -p apex-engine --test task_retention -- --nocapture \
  && cargo test -p apex-engine --test task_limits    -- --nocapture \
  && make gate
```

All green, with the printed numbers from §8 recorded in the pull request per the constitution's
evidence rule, the six negative checks in §9 each confirmed to have the fixture condition that
lets them fail, and the six mutations in §10 each confirmed to fail before being reverted. A claim
that something passes is accompanied by the command and its output, or it is not a claim.

And one thing that is **not** done until it is written down: the four quantities in *Known gaps* 1.
Until plan.md names them, four of the seven measurements above have nothing to be measured against,
and a suite that prints a number with no bound beside it is a suite that cannot fail.
