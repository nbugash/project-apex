# Research: Application Shell

**Branch**: `001-app-shell` | **Date**: 2026-09-21 | **Plan**: [plan.md](./plan.md)

Decisions taken during Phase 0. Each closes an alternative that was genuinely open. Two are
marked for promotion to Appendix A of the system specification, because they bind features
beyond this one.

---

## Layout persistence mechanism

**Promote to Appendix A.** This decides where session state lives for every later feature.

**Decision**: A single JSON file in the platform application-data directory, written with
`serde_json` through the `SessionStore` port. Not SQLite, and not the workspace cache database
that F003 introduces.

**Rationale**: The payload is small and structural — window geometry, three region sizes and
visibility flags, an ordered list of document references, one focus marker. It is read once at
launch and written on change. None of the properties that justify a database apply: no
querying, no concurrent writers, no partial reads, no growth with workspace size.

Keeping it out of the workspace cache also keeps the two lifetimes separate. The workspace
cache is a disposable projection of a remote source of truth and is expected to be evicted
(§5.5) or invalidated wholesale (§10.4). Interface state is neither — losing it because a
cache was cleared would be a defect. Storing them together would couple a durable user
preference to a throwaway cache, and would make F000 depend on F003, which sits behind
`[OPEN: H-BOOT]` and cannot be built yet.

**Alternatives considered**:

- *The F003 SQLite workspace cache.* Rejected on the coupling above, and because it would
  make this feature unbuildable until a blocked feature completes.
- *`tauri-plugin-store`.* A reasonable fit that does roughly this, but adds a dependency and
  a plugin permission for something `serde_json` plus a path does in a few lines. Reconsider
  if the state grows to need migrations or change notification.
- *Platform-native preference stores* (`plist` on macOS, XDG config on Linux). Rejected
  because it splits one behaviour across two implementations and two test paths for no user
  benefit.

---

## End-to-end testing on macOS

**Promote to Appendix A.** Every feature with interface surface inherits this limitation.

**Decision**: End-to-end tests run on Linux in CI via `tauri-driver` and WebdriverIO. macOS
gets a scripted smoke check — launch, wait for the ready signal, capture a screenshot, assert
the process exits cleanly — and is otherwise covered by unit and integration tests.

**Rationale**: `tauri-driver` works by delegating to the platform's WebDriver implementation.
Linux has `WebKitWebDriver`; macOS has no WebDriver for WKWebView. This is a missing
capability in the platform, not a configuration problem, so no amount of setup closes it.

The coverage loss is smaller than it appears. The interface layer is identical across both
platforms, so the user journeys in the specification exercise the same code either way. What
genuinely differs is window geometry behaviour and appearance, and both are reachable through
integration tests in the core plus the screenshot check.

Recorded as a Principle VII limitation in the plan's Complexity Tracking rather than silently
absorbed.

**Alternatives considered**:

- *Drive macOS through AppleScript or the accessibility APIs.* Technically possible, but a
  bespoke harness that needs its own maintenance, for a feature whose platform-specific
  surface is two behaviours.
- *Skip end-to-end everywhere for parity.* Rejected outright. Symmetry is not a reason to
  have less coverage; it would leave all four user journeys unverified on every platform.
- *Treat the screenshot check as an end-to-end test.* Rejected as mislabelling. It proves the
  application starts and paints; it does not exercise a journey.

---

## Preventing a light or unstyled first frame

**Decision**: Create the window with `visible: false` and a background colour set to the
design system's ground value in `tauri.conf.json`. The interface layer signals readiness
through the inbound command adapter once the stylesheet has applied and the first render has
committed; the core then shows the window.

**Rationale**: FR-020 forbids a light or unstyled frame at any point during launch, and the
default sequence violates it — the native window is mapped before the webview has parsed CSS,
so the user sees a system-coloured rectangle, then an unstyled document, then the styled
interface. Setting the window background alone fixes the first flash but not the second.
Gating visibility on an explicit readiness signal is what makes the requirement testable
rather than timing-dependent.

This also gives SC-011 something to assert against: with the window hidden until ready, "no
light frame observed" is a property of the design rather than a race that usually resolves in
the application's favour.

**Alternatives considered**:

- *Background colour only.* Insufficient; leaves the unstyled-document frame.
- *A fixed startup delay before showing.* Rejected — it trades a correctness property for a
  guess, and penalises fast machines to protect slow ones.
- *A splash window.* More moving parts than the problem needs, and a second window to style,
  persist and dismiss.

---

## Region layout implementation

**Decision**: Hand-rolled with CSS Grid and pointer-driven splitters. No docking framework.

**Rationale**: The specification requires regions that resize, hide, show and enforce a
minimum size. It explicitly excludes floating and detachable panels (Assumptions). That is a
grid with draggable dividers and a clamp — a small amount of code with no library semantics to
learn or fight.

Principle I is the deciding factor. A docking framework ships its own DOM structure and
stylesheet, and conforming it to the signed-off design means overriding its styles, which the
Design System Compliance section forbids. Adopting one would mean importing a large surface in
order to suppress most of it.

**Alternatives considered**:

- *Dockview, Golden Layout, or similar.* Rejected on the styling conflict above and on scope:
  their main value is floating and detachable panels, which this feature does not build.
- *A splitter component library.* Closer to the need, but the remaining logic — minimum-size
  clamping and visibility — is the part that carries the requirements, and it would still need
  writing and styling.

Revisit if detachable panels are ever signed off; that is the point at which a framework
starts earning its cost.

---

## Tab overflow handling

**Decision**: The tab strip scrolls horizontally, with an overflow control listing every open
document and scrolling the selected one into view.

**Rationale**: FR-006 requires every tab to stay reachable when more are open than fit. Scroll
alone satisfies reachability literally but poorly — finding one tab among forty means dragging
through them. The overflow list turns it into one interaction and gives the keyboard path a
natural target for FR-018.

**Alternatives considered**:

- *Shrink tabs to fit.* Rejected; labels become unreadable, which satisfies the letter of the
  requirement and defeats its purpose.
- *Wrap to multiple rows.* Rejected; the strip height then changes with tab count, moving the
  document area unpredictably.

---

## Design system integration

**Decision**: The build copies the design system from `mockups/` into the interface layer's
asset tree. It is never hand-authored, edited, or partially transcribed. `app.css` imports it
and defines nothing of its own.

**Rationale**: Principle I makes the signed-off artifact binding verbatim. Copying at build
time means the only way for the application to diverge is for someone to change the source
artifact, which is a visible, reviewable act. Transcribing tokens by hand would make drift
silent and invisible, which is precisely the failure the principle exists to prevent.

**Alternatives considered**:

- *Hand-transcribe tokens into the application's own stylesheet.* Rejected; guarantees drift.
- *Reference `mockups/` directly at runtime.* Rejected; couples the shipped bundle to a
  directory that exists for design review, and would break packaging.

---

## Design adherence lint

**Decision**: Port the adherence configuration from its React plugin setup to the interface
layer's actual toolchain, preserving every rule it expresses, and run it in CI as a required
check.

**Rationale**: The Design System Compliance section requires this lint in CI, and notes it is
currently configured for React while the interface layer is Svelte. The rules themselves — no
raw hex, no raw pixel values, no hard-coded font families — are framework-agnostic; only the
plugin wiring is not. This feature introduces the first styled surfaces, so it is the point at
which the lint becomes meaningful and the point at which its absence would start accumulating
violations.

**Alternatives considered**:

- *Defer to F011 with packaging.* Rejected; every surface built before then would go
  unchecked, and retrofitting a lint against existing violations is materially harder than
  keeping a clean tree clean.
- *Manual design review instead.* Rejected; Principle I's token discipline exists specifically
  to make fidelity machine-checkable rather than a matter of argument.

---

## Window geometry restoration across displays

**Decision**: On launch, restore the saved geometry only after intersecting it with the
currently attached monitors. If the saved rectangle does not meaningfully overlap any attached
display, fall back to a default position on the primary display.

**Rationale**: FR-009 requires the window to open on an attached display, and SC-009 makes it
measurable. The common failure is a laptop last docked to an external monitor: the saved
coordinates are valid numbers that place the window somewhere the user cannot see or reach.
Intersecting against attached monitors turns an invisible failure into a visible window.

"Meaningfully overlap" needs a concrete threshold so the requirement is testable rather than
judgemental; the title bar must be within an attached display's working area, since a window
whose title bar is off-screen cannot be dragged back.

**Alternatives considered**:

- *Always open on the primary display.* Rejected; discards a preference that is correct in the
  common case of an unchanged setup.
- *Restore coordinates unconditionally and let the window manager cope.* Rejected; behaviour
  differs across macOS and the various Linux compositors, which is exactly the inconsistency
  the requirement exists to remove.
