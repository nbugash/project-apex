# icons/

Phosphor Icons 2.1.1, regular weight, vendored for offline use. MIT — see LICENSE.txt.

| File | Size | What |
| --- | --- | --- |
| Phosphor.woff2 | 144 KB | The icon font — all ~1,530 regular glyphs |
| phosphor.css | 78 KB | Full stylesheet, `src` trimmed to woff2 only |
| phosphor-subset.css | 4 KB | Only the 58 glyph classes the Apex mockup uses |

## Use

```html
<link rel="stylesheet" href="icons/phosphor-subset.css">
```

Then `<i class="ph ph-git-branch"></i>`. Size with `font-size`, color with `color` — it's a font, so icons inherit text color and align on the baseline.

Use `phosphor.css` instead if you'll add icons beyond the mockup's 58; the woff2 is identical either way, so the subset only saves stylesheet bytes, not font bytes.

## The 58 in use

```
arrow-down arrow-line-down arrow-line-up arrows-clockwise brackets-curly
bug caret-double-left caret-down caret-right check-square
clock-counter-clockwise cloud cloud-check coffee columns
command crosshair cube database desktop
dot-outline file-code file-magnifying-glass file-text folder
folder-open function gauge gear-six git-branch
info keyboard list-dashes magnifying-glass minus
package palette pause pencil-simple play
play-circle plus power puzzle-piece scissors
shipping-container sidebar-simple sliders-horizontal spinner stop
terminal-window test-tube text-aa tree-structure warning-circle
wrench x x-circle
```

## Other weights

Only `regular` is vendored. Phosphor also ships thin, light, bold, fill and duotone — each a separate font file and stylesheet under `@phosphor-icons/web@2.1.1/src/<weight>/`. Fetch one the same way if you want a heavier toolbar.
