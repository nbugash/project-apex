# Quickstart: Shell Chrome Fidelity

**Branch**: `002-shell-chrome-fidelity` | **Date**: 2026-09-21 | **Plan**: [plan.md](./plan.md)

How to run the chrome and prove it matches the prototype. Scenarios map to the user stories
and measurable outcomes in [spec.md](./spec.md). Entity shapes are in
[data-model.md](./data-model.md); the added commands are in
[contracts/rail-commands.md](./contracts/rail-commands.md).

Everything in the shell's own quickstart still applies — this adds to it rather than
replacing it. See [`../001-app-shell/quickstart.md`](../001-app-shell/quickstart.md).

---

## Prerequisites

Those of the shell, plus:

| Requirement | Notes |
|-------------|-------|
| ImageMagick | Already required for end-to-end screenshot capture. `compare` provides the pixel check |

No new prerequisite. The fidelity gate deliberately reuses what the end-to-end harness
already needs.

---

## Run

```bash
npm run ds:sync     # also generates the prototype's layout tokens
npm run tauri dev
```

Expected: a window chrome header with the product mark and project name, an activity rail
down the left edge, and a tool window beside it with its header row.

---

## Validation scenarios

### 1. The chrome matches the prototype (User Story 1, SC-001)

```bash
npm run gate:fidelity
```

**Expected**: passes. It performs both checks — surface geometry within 2 device pixels, and
rendered pixels within 0.5% of the baseline at 1200×800.

To confirm the gate can actually fail, change a chrome dimension and run it again. It must
fail, name the surface that moved, and leave a difference image. A gate that has never been
seen to fail has not been tested.

### 2. Every dimension is a token (SC-002)

```bash
npm run lint:ds
```

**Expected**: passes, and fails if a prototype dimension is written as a literal. The layout
tokens are generated from the prototype by `ds:sync`, so a value can only change by changing
the prototype.

### 3. Rail navigation (User Story 2)

Select each available destination. The tool window switches and the rail marks the active one.
Select the active destination again: the panel collapses. Select it once more: it returns at
the width it had, not a default.

**Expected**: an unavailable destination is visibly unavailable and does nothing when chosen —
never present and silently inert.

### 4. Keyboard reachability (SC-003)

```bash
xvfb-run -a npm run e2e -- --spec tests/e2e/chrome-fidelity.spec.ts
```

**Expected**: every destination is reachable and activatable by keyboard alone, and the
focused element shows the design system's focus ring rather than a platform default.

### 5. Greyscale distinguishability (SC-004)

With the display rendered in greyscale, confirm the active destination is still identifiable,
and that an unavailable destination still reads as unavailable.

### 6. Tool window state survives restart (SC-005)

Select a destination, resize the panel, collapse it, quit, relaunch.

**Expected**: the same destination is active, the panel is still collapsed, and expanding it
returns the width you set.

### 7. An existing session survives the upgrade (data-model, Migration)

```bash
# With a session file written by the previous version still in place:
npm run tauri dev
```

**Expected**: window geometry, layout and open tabs are all preserved, and the tool window
appears with its defaults. **The session is not discarded.** This is the scenario the
migration decision exists for — without it, shipping this feature would silently erase every
user's layout.

### 8. The gate refuses to pass when it cannot run (FR-015)

```bash
mv tools/gate-fidelity/reference/shell.json /tmp/ && npm run gate:fidelity
```

**Expected**: reports an error about the missing baseline and exits non-zero. It must not
pass. Restore the file afterwards.

---

## Updating the baseline

```bash
npm run gate:fidelity:update
```

Only when the approved design has actually changed. This writes the golden file, and the
resulting diff is the thing a reviewer should look at — it shows the approved appearance
changing. The comparison command never writes the baseline itself; a gate that regenerates
what it judges against passes unconditionally.

---

## Automated suites

```bash
cargo test --manifest-path src-tauri/Cargo.toml    # includes the migration test
npm run test:unit
npm run lint:ds
npm run gate:fidelity                              # CI-required
xvfb-run -a npm run e2e                            # Linux only
```

The fidelity gate runs where the end-to-end suite runs, for the same reason: it needs a
rendered window. macOS keeps the existing smoke check — see Appendix A, A-E2E.
