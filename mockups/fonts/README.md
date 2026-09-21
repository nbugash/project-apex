# fonts/

Inter, vendored for offline use. Files from [Fontsource](https://fontsource.org/fonts/inter) (`inter@latest`), SIL OFL 1.1 — see LICENSE.txt.

| File | Weight | Subset |
| --- | --- | --- |
| inter-latin-400.woff2 | 400 | latin |
| inter-latin-ext-400.woff2 | 400 | latin-ext |
| inter-latin-500.woff2 | 500 | latin |
| inter-latin-ext-500.woff2 | 500 | latin-ext |
| inter-latin-600.woff2 | 600 | latin |
| inter-latin-ext-600.woff2 | 600 | latin-ext |
| inter-latin-700.woff2 | 700 | latin |
| inter-latin-ext-700.woff2 | 700 | latin-ext |

~240 KB total. `inter.css` carries the eight `@font-face` rules with the correct `unicode-range` per subset, so the browser downloads latin-ext only when a page actually needs it.

## Use

```html
<link rel="stylesheet" href="fonts/inter.css">
```

Load it **after** the design system's `styles.css`. Nocturne's stylesheet opens with a Google Fonts `@import` for the same family; the later local declarations win, and the remote import simply fails harmlessly with no network. To drop the remote request entirely, delete line 2 of `_ds/nocturne-…/styles.css`:

```css
@import url('https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap');
```

The tokens `--font-heading` / `--font-body` already resolve to `"Inter", system-ui, sans-serif` — nothing else to change.

## Code font — JetBrains Mono

| File | Weight | Subset |
| --- | --- | --- |
| jetbrains-mono-latin-{400,500,700}.woff2 | each | latin |
| jetbrains-mono-latin-ext-{400,500,700}.woff2 | each | latin-ext |

~87 KB total, declared in the same `inter.css`. The design's stack is `'JetBrains Mono', ui-monospace, Menlo, monospace` — the platform fallbacks stay so it degrades gracefully. SIL OFL 1.1, same as Inter.

`ttf/JetBrainsMono-Variable.ttf` (183 KB, weight axis 100–800) is there to install locally so design tools match.
