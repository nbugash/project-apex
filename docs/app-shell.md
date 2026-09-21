# Application Shell (F000)

What later features need to know about the shell they mount into. The specification lives in
`specs/001-app-shell/`; this is the working map.

## Shape

Two runtimes in one process. The Rust core owns state, persistence and the window; the
webview owns pixels. **The core owns truth and the webview owns pixels** is the invariant —
the webview holds no authoritative state and can be reloaded without loss.

```
src-tauri/src/
  domain/        no external imports at all
  application/   ports + use cases; depends only on domain and ports
  adapters/      inbound (Tauri commands), outbound (store, connection)
  composition.rs the only place adapters are bound
```

Dependencies point inward. Nothing in `domain` or `application` imports Tauri; if you find
yourself wanting to, the boundary is in the wrong place.

## Extending it

**Adding a command.** Add the signature to `contracts/shell-commands.md` first, then the
handler in `adapters/inbound/tauri_commands.rs`, then register it in `lib.rs`. Validate
arguments in the handler: input from the webview is untrusted regardless of what the
interface layer already checked (Principle VI).

**Adding a side effect.** It becomes an outbound port in `application/ports/`, named for the
capability rather than the technology — `SessionStore`, not `JsonFileStore`. Implement it in
`adapters/outbound/` and bind it in `composition.rs`.

**Replacing the connection source (F001).** `StubConnectionStatusSource` implements
`ConnectionStatusSource`. Swap the binding in `composition.rs`; nothing else changes. That
one-line swap is the property the port structure exists to buy.

## Things that will bite you

**The window is created hidden.** It becomes visible only when the interface calls
`shell_ready`. If you add startup work before that call, the window stays dark for longer;
if you remove the call, it never appears. This is deliberate — it makes "no unstyled frame"
(FR-020) structural rather than a race.

**Session state is not the workspace cache.** It lives in its own JSON file for reasons
recorded in Appendix A, A-STATE. Do not move it into the SQLite cache F003 introduces: that
cache is disposable and evicted, and interface state is not.

**Persistence is debounced.** Mutations return immediately and a writer thread coalesces
them. A failed write is reported, never propagated to the caller (FR-023). Do not add an
await on persistence to an interaction path.

**Styling comes from `mockups/`.** `npm run ds:sync` copies it; `src/lib/ds/` is build output
and is gitignored. Never hand-edit a token value. `npm run lint:ds` fails on raw hex, raw px
beyond 1-2px hairlines, and hard-coded fonts; `ds:sync` fails on a reference to a token the
design system does not define, which is the FR-022 designer-gap signal.

**Known design gap.** The system defines `--font-heading` and `--font-body` (both Inter) but
no mono token, although JetBrains Mono is bundled. The editor feature will need one; that is
a designer conversation, not an implementation choice.

## Running it

```bash
npm install && npm run ds:sync
npm run tauri dev          # development
cargo test --manifest-path src-tauri/Cargo.toml
npm run test:unit
npm run lint:ds            # required in CI
npm run perf:budget        # required in CI
xvfb-run -a npm run e2e    # Linux only; see Appendix A, A-E2E
```

End-to-end runs on Linux only: macOS provides no WebDriver for its platform webview. macOS
gets `npm run smoke:macos`, which proves the app starts, paints and exits cleanly — it
exercises no user journey and is not a substitute.

`APEX_DATA_DIR` overrides the profile directory. The end-to-end suite uses it to give each
run an isolated profile it can seed and corrupt.
