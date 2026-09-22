# Fidelity baseline: where the numbers came from

**Derived on**: 2026-09-22 · **Reference size**: 1200 × 800 · **Source**:
`mockups/Apex IDE (standalone).html`

This file records how `geometry.json` and `baseline-1200x800.png` were produced, and the
reconciliation between them. It exists because a baseline nobody can trace is a baseline
nobody can trust: the next person to see the gate fail needs to know whether the number it
is defending came from the approved design or from whatever the build happened to render.

## Why the geometry is derived from the prototype, not from our build

Capturing the expected geometry from the application would make the gate a self-consistency
check. It would lock in whatever had been implemented — including drift already present —
and thereafter catch only *future* drift, reporting green against a design it had never
actually matched.

That is not hypothetical. When these numbers were first compared against the
implementation, two of the three surfaces were wrong (see Reconciliation below). A baseline
captured from our own output would have recorded those wrong values as correct and the gate
would have passed on day one.

So the numbers come from rendering the signed-off prototype and measuring it.

## How

```bash
node tools/gate-fidelity/derive.mjs \
  --playwright <path to a playwright module> \
  --chromium   <path to a chromium binary>
```

`derive.mjs` loads the prototype at the reference size, waits for it to apply its density
preset — the prototype sets its `--vk-*` custom properties from script on mount, so
measuring earlier records the CSS fallback values, which belong to a different preset —
then reads `getBoundingClientRect()` for each surface and writes `geometry.json`.

The browser engine is passed in rather than depended on. Deriving happens a handful of
times, when the prototype changes; carrying a browser download in this project's
dependencies for that would cost every developer and every CI run.

### Surfaces, and how each is addressed

The prototype's markup is generated and carries no class names, so it is addressed by
element. The shell is addressed by the class its component declares.

| Surface | In the prototype | In the shell |
|---|---|---|
| chrome header | `header` | `header.chrome` |
| activity rail | `nav` | `nav.rail` |
| tool window | `aside` | `aside.tool-window` |

These are the three surfaces SC-001 names.

### Measured

| Surface | Position | Size |
|---|---|---|
| chrome header | 0, 0 | 1200 × 46 |
| activity rail | 0, 46 | 44 × 728 |
| tool window | 44, 46 | 310 × 728 |

Also observed, and **not** gated, because it belongs to the feature that owns it rather
than to F018: the prototype's status bar is `1200 × 26` at `0, 774`. It is recorded here
because the body height — and therefore the rail's and the tool window's heights — is
whatever the chrome header and status bar leave behind. A change to the status bar moves
two gated surfaces.

## Reconciliation (T041)

The implementation was checked against the derived numbers before any baseline image was
approved. `update.mjs` refuses to approve a rendering that differs, so this reconciliation
is enforced rather than remembered.

**First run — refused:**

```
activity rail: height is 737, the prototype has 728
tool window:   height is 737, the prototype has 728
```

**Cause**: the status bar. F000 built it from the generic `--space-*` scale, which put it at
17 px against the prototype's 26 px. Nine pixels at the bottom of the window is invisible to
the eye, and it made both gated surfaces nine pixels too tall.

**Fix**: the status bar's own metrics are now extracted from the prototype by `ds:sync`
(`--vk-status-height`, `--vk-status-gap`, `--vk-status-pad`, `--vk-status-size`,
`--vk-status-icon`) and applied in `src/lib/statusbar/StatusBar.svelte`. This is the same
treatment every other chrome dimension gets, so the value cannot drift back.

**Second run — approved.** All three surfaces within the 2 px position tolerance.

This is the reconciliation the task asked for, and it found a real defect in shipped code
that every other gate in the project had passed.

## The baseline image is a different thing

`baseline-1200x800.png` is a rendering of **this application**, approved once its geometry
had been reconciled above. It is the regression half of the gate: it catches a change in
what the shell paints, within the area tolerance.

It is deliberately not a picture of the prototype. The prototype is a populated mock with a
file tree, an editor, a terminal and a completion popup; the shell is mostly empty. A pixel
comparison against it would be red on every run from the first, and a gate that is always
red gets switched off.

## Re-deriving

Re-derive when the prototype changes:

1. `node tools/gate-fidelity/derive.mjs --playwright … --chromium …`
2. `npm run gate:fidelity` — expect failures naming the surfaces that moved
3. Change the implementation until it matches
4. `npm run gate:fidelity:update` — refuses while any surface still differs
5. Commit `geometry.json`, `baseline-1200x800.png` and an update to this file

Never edit `geometry.json` by hand to make the gate pass. The numbers are the design's, and
editing them is how a fidelity gate quietly becomes a formality.
