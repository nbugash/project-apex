# Quickstart: Application Shell

**Branch**: `001-app-shell` | **Date**: 2026-09-21 | **Plan**: [plan.md](./plan.md)

How to run the shell and prove it satisfies its specification. Scenarios map to the user
stories and measurable outcomes in [spec.md](./spec.md). Entity shapes are in
[data-model.md](./data-model.md); the command surface is in
[contracts/shell-commands.md](./contracts/shell-commands.md).

---

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| Rust 1.75+ | `rustup` toolchain, stable channel |
| Node 20+ | For the interface layer build |
| Tauri v2 system dependencies | macOS: Xcode command line tools. Linux: `webkit2gtk-4.1`, `libayatana-appindicator3`, `librsvg2` |
| `WebKitWebDriver` | Linux only, required for end-to-end. Usually packaged as `webkit2gtk-driver` |

macOS has no WebDriver for its platform webview, so the end-to-end suite does not run there.
See [research.md](./research.md), "End-to-end testing on macOS".

---

## Setup

```bash
npm install
npm run ds:sync          # copies the signed-off design system from mockups/ into the asset tree
cargo build --manifest-path src-tauri/Cargo.toml
```

`ds:sync` is not optional. The interface layer imports the design system from its copied
location, and the build fails rather than falling back to unstyled output — Principle I makes
transcribing or approximating the design a violation, so an absent design system must be an
error, not a degraded mode.

---

## Run

```bash
npm run tauri dev
```

Expected: a window appears already in the approved dark appearance, with a navigation region,
a document area, and an output region. The status area shows a workspace name and a
connection state driven by the stub adapter.

---

## Validation scenarios

### 1. Layout persists across restart (User Story 1, SC-002)

```bash
npm run tauri dev
```

Widen the navigation region, hide the output region, resize the window. Quit. Relaunch.

**Expected**: region sizes, region visibility, and window geometry are exactly as left.

### 2. Corrupt state does not prevent launch (User Story 1, SC-003)

```bash
# macOS
printf 'not json' > "$HOME/Library/Application Support/<app-id>/session.json"
# Linux
printf 'not json' > "${XDG_DATA_HOME:-$HOME/.local/share}/<app-id>/session.json"

npm run tauri dev
```

**Expected**: the window opens with the default layout, no error dialog, and no message
implying something went wrong. Falling back is a normal path. Repeat with a truncated file, an
empty file, and a file whose `focused_document_id` names a document absent from `documents` —
all four are discarded in full rather than partially recovered.

### 3. Tabs restore with focus (User Story 2, SC-002)

Open several documents, reorder them by dragging, focus one in the middle, quit, relaunch.

**Expected**: the same documents, in the same order, with the same one focused.

### 4. Every tab stays reachable (User Story 2, FR-006)

Open more documents than fit the window width.

**Expected**: the strip scrolls, and the overflow control lists every open document and
scrolls the selected one into view. No tab becomes unreachable, and tab labels do not shrink
to illegibility.

### 5. Connection state is visible and non-colour-dependent (User Story 3, SC-006, SC-007)

Drive the stub through its transitions:

```bash
npm run stub:connection -- --state connecting
npm run stub:connection -- --state connected
npm run stub:connection -- --state disconnected
```

**Expected**: the status area reflects each transition within 5 seconds without user action.
Repeat with the display in greyscale (macOS: Accessibility → Display → Colour Filters;
Linux: a greyscale compositor filter) and confirm all four states remain distinguishable.

### 6. No light or unstyled frame during launch (User Story 4, SC-011)

Set the operating system to light appearance, then:

```bash
npm run e2e -- --spec launch-appearance      # Linux: captures frames from window creation
```

**Expected**: no captured frame shows a light or unstyled state. Repeat with the OS set to
dark. On macOS, run the smoke check and inspect the captured screenshot manually:

```bash
npm run smoke:macos
```

### 7. Window returns to an attached display (SC-009)

With an external display attached, move the window onto it and quit. Detach the display.
Relaunch.

**Expected**: the window opens fully on the remaining display.

### 8. The interface stays responsive during background work (SC-005)

```bash
npm run stub:load -- --duration 10s
```

While it runs, drag a splitter, switch tabs, resize the window.

**Expected**: no interaction stalls. Nothing queues behind the background task.

---

## Automated suites

```bash
cargo test --manifest-path src-tauri/Cargo.toml    # unit + integration
npm run test:unit                                  # interface-layer unit
npm run e2e                                        # Linux only
npm run lint:ds                                    # design adherence — CI-required
npm run perf:budget                                # asserts SC-001 and SC-004
```

`lint:ds` and `perf:budget` are required checks, not advisory. The first enforces Principle I
mechanically; the second is what makes Principle V a measurement rather than a claim.

---

## Known coverage gap

End-to-end scenarios 1 through 8 run on Linux. On macOS, scenarios 1, 2, 3, 6 and 7 are
covered by integration tests in the core plus the smoke check; scenarios 4, 5 and 8 are
verified manually until a WebDriver implementation exists for the platform webview. Recorded
in the plan's Complexity Tracking.
