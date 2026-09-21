# Apex IDE mockup — asset & token manifest

Every external dependency, font, icon, color and measurement used by `Apex IDE.dc.html`.
Design system: **Nocturne** (`_ds/nocturne-85583080-aa1c-49ca-9469-934d1899d563/`).

---

## 1. External dependencies

| What | Source | Purpose |
| --- | --- | --- |
| Nocturne `styles.css` | `_ds/nocturne-…/styles.css` (local) | All design tokens + component classes (`.btn`, `.input`, `.card`, `.tag`, `.field`, `.seg`, `.radio`, `.elev-*`) |
| Nocturne bundle | `_ds/nocturne-…/_ds_bundle.js` (local) | Design-system component runtime |
| Inter | `fonts/inter.css` + 8 woff2 files (local, vendored) | All UI text |
| JetBrains Mono | `fonts/inter.css` + 6 woff2 files (local, vendored) | All code, terminal, numeric readouts |
| Phosphor Icons 2.1.1 (regular) | `icons/phosphor-subset.css` + `icons/Phosphor.woff2` (local, vendored) | All 58 icons |

Both were CDN links and are now vendored into the project — see `fonts/README.md` and `icons/README.md`. The design system's `styles.css` still opens with a Google Fonts `@import` for Inter; the local `@font-face` rules load after it and win, so the remote request is redundant (delete line 2 of `styles.css` to drop it entirely).

**Not used:** Tailwind CSS, Bootstrap, any component library, any SVG file, any raster image, any icon sprite. There is no JavaScript framework dependency beyond the design-system bundle. No web fonts beyond Inter.

---

## 2. Typography

### UI type — Inter

`--font-heading` and `--font-body` are both `"Inter", system-ui, sans-serif`. Headings use weight **500** (`--font-heading-weight`) with `letter-spacing: -0.015em`; body is **400**. Nocturne forbids bolding headings past 500 — hierarchy is size and space.

| Role | Size | Weight | Notes |
| --- | --- | --- | --- |
| Wordmark "APEX" | 11px | 500 | uppercase, `letter-spacing: .18em` |
| Dialog / popover titles | 14–14.5px | 500 | `--font-heading` |
| Toolbar buttons, tabs, tree rows | 12–12.5px | 400 | |
| Body / base UI | 13px (`--vk-fs`) | 400 | 12.5px compact, 13.5px roomy |
| Panel headers | 10.5px | 400 | uppercase, `letter-spacing: .11em` |
| Section labels in dialogs | 10.5px | 400 | uppercase, `letter-spacing: .1em` |
| Captions, spec pins, footnotes | 10.5–11px | 400 | |

### Code type — JetBrains Mono

`'JetBrains Mono', ui-monospace, Menlo, monospace` — vendored in `fonts/`, weights 400/500/700, latin + latin-ext. Pinned deliberately: code type is the surface a developer stares at all day, and letting it vary by platform meant the mockup rendered differently on every machine. Designed for code — tall x-height, 1.2 line-height baked into its metrics, disambiguated `0 O o` / `1 l I` / `rn m`.

Used for: editor buffer, line numbers, TOML, terminal output, tree-sitter queries, diff bodies, branch/RTT/latency figures, variable values, keyboard shortcut chips, file paths, capability chips.

| Role | Size | Line height |
| --- | --- | --- |
| Editor / diff body | 13px (`--vk-code`) | 21px (`--vk-line`) |
| Terminal | 12.5px | 1.6 |
| Completion rows, variables, results | 12–12.5px | — |
| Status-bar figures, chips | 10–11px | — |

> Fallbacks stay in the stack so the design degrades to the platform mono if the webfont fails. JetBrains Mono runs slightly wider than SF Mono at the same px size — the 21px line height and the editor's column widths already account for it.

---

## 3. Icons — Phosphor, regular weight, 58 glyphs

Rendered as an icon font: `<i class="ph ph-{name}">`, sized with `font-size` (8px–17px), colored with `color`.

**Toolbar & chrome:** `cube` · `caret-down` · `play` · `play-circle` · `bug` · `magnifying-glass` · `crosshair` · `cloud` · `cloud-check` · `desktop` · `sliders-horizontal`

**Rail & tool windows:** `folder-open` · `list-dashes` · `git-branch` · `clock-counter-clockwise` · `puzzle-piece` · `sidebar-simple` · `caret-double-left` · `folder` · `package` · `file-code` · `file-text` · `gear-six` · `check-square` · `spinner`

**Editor & breadcrumbs:** `caret-right` · `function` · `brackets-curly` · `database` · `keyboard` · `tree-structure` · `columns`

**Dock:** `terminal-window` · `warning-circle` · `gauge` · `x-circle` · `info` · `pause` · `arrow-down` · `arrow-line-down` · `arrow-line-up` · `stop` · `dot-outline`

**Dialogs, packs, run configs:** `command` · `file-magnifying-glass` · `pencil-simple` · `plus` · `minus` · `x` · `power` · `arrows-clockwise` · `scissors` · `wrench` · `test-tube` · `coffee` · `palette` · `text-aa` · `shipping-container`

---

## 4. Color

### Nocturne tokens (from `styles.css`)

| Token | Value | Used for |
| --- | --- | --- |
| `--color-bg` | `#161826` | Editor ground, inset fields |
| `--color-surface` | `#232532` | Toolbar, tab strip, status bar, popovers, dialogs |
| `--color-neutral-900` | `#292b31` | Tool-window and dock grounds |
| `--color-text` | `#e9e9ed` | Primary text |
| `--color-accent` | `#9184d9` | Accent lines, marks, active states, caret |

Tonal ramps in use — **neutral** 200 `#e4e7f5`, 300 `#cfd3e5`, 400 `#b2b6ca`, 500 `#9397ab`, 600 `#75798c`, 700 `#595d6c`, 800 `#3f424d`, 900 `#292b31`; **accent** 100 `#f5f4ff`, 200 `#e7e5fe`, 300 `#d2cefd`, 400 `#b5abfc`, 600 `#796cbf`, 700 `#5d5294`, 800 `#423a6a`, 900 `#2b2741`.

Muted text and hairlines are `color-mix(in srgb, var(--color-text) N%, transparent)`. **Text** uses N of 52, 55, 58, 60 or 62 only — Nocturne's muted band, ≥3.6:1 on `--color-bg`; anything lower fails the system's own floor. **Non-text** (hairlines, hovers, progress tracks, toggle borders) uses N of 6/8/9/10/12/14/20. Line numbers are `--color-neutral-600` (#75798c, 3.65:1) — `-700` is too dark to read at 13px. Separators are `box-shadow: inset` hairlines rather than borders, so they don't affect layout.

### Added outside the system — 5 semantic colors

The design system is mono (accent only), so state colors and syntax needed values it doesn't carry. All are low-chroma to match Nocturne's discipline:

| Hex | Role |
| --- | --- |
| `#d4736a` | Error — diagnostics, breakpoint dot, removed diff lines, wavy underline |
| `#c9a96a` | Warning — modified-file marks, stale/cancelled labels, over-budget latency |
| `#7fa98f` | Success — tests passed, added diff lines, healthy server dots |
| `#8fb3a5` | Go-file tint (secondary language accent) |
| `#0d0f18` | Modal scrim base, used at 62% via `color-mix` |

### Syntax palette (derived from the neutral/accent ramps)

| Hex | Scope |
| --- | --- |
| `#b5abfc` (accent-400) | Keywords, annotations, TOML section headers |
| `#d2cefd` (accent-300) | Types, classes, doc signatures |
| `#e4e7f5` (neutral-200) | Function and method names |
| `#cfd3e5` (neutral-300) | Identifiers, numbers, terminal output |
| `#b2b6ca` (neutral-400) | String literals |
| `#9397ab` (neutral-500) | Punctuation, operators |
| `#75798c` (neutral-600) | Comments |
| `#595d6c` (neutral-700) | Line numbers |
| `#e6e6ea` | Inlay hints (virtual text) |

---

## 5. Geometry

**Radii** — `--radius-sm` 4px (buttons, rows, chips) · `--radius-md` 8px (popovers, cards, inputs) · `--radius-lg` 14px (modals, palette).

**Elevation** — `--shadow-md` `0 0 0 1px #595d6c, 0 6px 18px rgba(0,0,0,.55)` for popovers and the latency HUD; `--shadow-lg` `0 0 0 1px #9397ab, 0 16px 40px rgba(0,0,0,.65)` for modals and the command palette. Never stacked.

**Fixed chrome heights** — toolbar 46px · remote banner 26px · tab strip 34px · breadcrumbs 24px · dock header 30px · dock tab strip 28px · status bar 26px · rail width 44px.

**Density variables** (set on `:root` by the `density` prop):

| Variable | compact | default | roomy |
| --- | --- | --- | --- |
| `--vk-line` (code line height) | 19px | 21px | 25px |
| `--vk-row` (list row height) | 22px | 25px | 30px |
| `--vk-fs` (UI font size) | 12.5px | 13px | 13.5px |
| `--vk-code` (code font size) | 12.5px | 13px | 13.5px |
| `--vk-tool` (tool window width) | 250px | 276px | 310px |
| `--vk-dock` (dock height) | 206px | 236px | 268px |

The dock is capped at `min(var(--vk-dock), 34vh)` so the editor always keeps room.

---

## 6. Motion

One keyframe only — `vkpulse` (opacity .35 → 1 → .35, 1.1s): the streaming-search spinner and the terminal caret. No transitions, no easing curves; everything else is instant, matching the doc's frame-budget stance.

---

## 7. Tweakable props (host Tweaks panel)

| Prop | Values | Default |
| --- | --- | --- |
| `density` | compact / default / roomy | default |
| `toolSide` | left / right | left |
| `perfReadout` | status / hud / off | status |
| `completionStyle` | list / detail | list |
| `showInlay` | boolean | true |
| `remoteBanner` | boolean | true |

---

## 8. Vendored assets

Both webfonts now live in the project, so the design has no runtime CDN dependency.

```
fonts/
  inter.css                    8 @font-face rules with per-subset unicode-range
  inter-latin-{400,500,600,700}.woff2
  inter-latin-ext-{400,500,600,700}.woff2
  jetbrains-mono-latin-{400,500,700}.woff2
  jetbrains-mono-latin-ext-{400,500,700}.woff2
  ttf/Inter-Variable.ttf       856 KB — 100–900 + opsz axis, full charset
  ttf/JetBrainsMono-Variable.ttf  183 KB — 100–800 weight axis
  ttf/inter-{latin,latin-ext}-{400,500,600,700}.ttf
  LICENSE.txt                  SIL OFL 1.1
  README.md  ·  ttf/README.md
icons/
  Phosphor.woff2               144 KB — all ~1,530 regular glyphs
  phosphor.css                 full stylesheet, woff2-only src
  phosphor-subset.css          only the 58 classes in use
  ttf/Phosphor.ttf             477 KB — same glyphs, for design tools
  LICENSE.txt                  MIT
  README.md  ·  ttf/README.md
```

`Apex IDE.dc.html` loads `fonts/inter.css` and `icons/phosphor-subset.css` — the woff2 set, ~480 KB (`inter.css` declares both families). The `ttf/` folders are **not** used by the mockup; they're for installing on your machine and for design tools that won't take woff2. Install `Inter-Variable.ttf` and `Phosphor.ttf` locally and Figma will render this design with the right type.

**Licensing** — Inter and JetBrains Mono are both SIL OFL 1.1 (free to bundle and serve; keep LICENSE.txt alongside). Phosphor is MIT. Both are safe to ship in a product.
