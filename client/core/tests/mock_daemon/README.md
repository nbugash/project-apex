# The mock remote engine

A stand-in for `ide-engine` that speaks `Content-Length` framing and **nothing above it**.

Built as the binary `apex-mock-daemon`. Integration tests reach it through
`MockSpawner`, which hands the transport a real child process on real pipes — the transport
under test is the production one, and only what is on the far end differs. That is the whole
purpose of the `ProcessSpawner` port.

## Why it implements no engine method

It answers every request with the same body, whatever was asked:

```json
{"jsonrpc":"2.0","id":"<the id you sent>","result":{"echo":true}}
```

Giving it real behaviour would make it a second implementation of the engine. Two
implementations drift, and the drift would be discovered by F002 — against a double that had
been passing for months, with every test written on top of it now suspect.

Restricting it to framing bounds that risk, because framing is the layer §4.1 defines
normatively and exactly. The mock and the real engine can be checked against the same text,
so agreement between them is a fact rather than a hope. `the_mock_implements_no_engine_method`
in `main.rs` enforces this: it fails if any §4.8 method name appears in this directory,
which is what stops engine behaviour arriving one reasonable-looking method at a time.

## Driving it

Behaviour is scripted through the `APEX_MOCK_SCRIPT` environment variable: a comma-separated
list of directives. `MockSpawner::new("delay=250,drop=20")` sets it.

| Directive | Effect | What it is for |
|---|---|---|
| `echo` | Reply to every request. The default. | The ordinary case. |
| `delay=<ms>` | Wait before each reply. | Latency, and making write order observable. |
| `drop=<n>` | Silently drop every nth reply. | Requests that must time out. `drop=1` answers nothing; `drop=20` is 5% loss. |
| `reorder=<n>` | Hold n replies, then emit them backwards. | Correlation. Positional matching fails every request here. |
| `malformed` | Reply with a body that is not JSON. | A frame the transport must discard without losing alignment. |
| `oversized` | Declare a length beyond the 1 MiB cap. | A refusal that must not consume the declared bytes. |
| `stall=<ms>` | Answer nothing, then close after `<ms>`. | A silently dead link. See the note below. |
| `close-mid-frame` | Write half a frame and exit. | A truncated stream at the worst moment. |
| `lossy` | Shorthand for `delay=250,drop=20`. | The feature map's link profile. |

Directives combine: `delay=100,drop=5` is a slow link that loses a fifth of its replies.
An unrecognised directive is reported on stderr and ignored, so a typo degrades to `echo`
rather than silently changing the test's meaning.

## The one thing this mock cannot model

There is no socket here, so there is no keepalive. `ServerAliveInterval` and
`ServerAliveCountMax` — the flags that decide whether a pulled cable is noticed in 45 seconds
or never — have no effect on anything in this directory.

That is why `stall` closes the pipe rather than merely going quiet. A real `ssh` whose
keepalive gives up **exits**, so the transport's single observation is EOF; there is no
second signal for it to wait for. A mock that went quiet without closing would model a
situation that cannot occur, and a test written against it would hang forever.

The keepalive flags themselves are asserted in `spawner.rs`'s unit tests, where their absence
is visible. No integration test in this suite can catch it.

## Adding a directive

Add a field to `Script`, a branch in `Script::from_env`, the behaviour in `main`, and a row
above. Keep it about framing, timing or the shape of the stream. A directive that makes the
mock understand a request body is the beginning of a second engine.
