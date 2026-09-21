# icons/ttf/

`Phosphor.ttf` — Phosphor Icons 2.1.1, regular weight, all ~1,530 glyphs as TrueType (477 KB).

For the web use `../Phosphor.woff2` instead: same glyphs, 144 KB. This TTF is for **design tools and native apps** — install it and the icons become available as a text font, so you can type/paste glyphs into Figma or a desktop UI toolkit rather than importing SVGs.

## Finding a glyph

Icons live in the Private Use Area, so they won't show in a normal character picker by name. Two ways:

1. Look the name up at https://phosphoricons.com, copy the glyph directly from the site.
2. Read the codepoint out of `../phosphor-subset.css` — each rule is `.ph-git-branch:before { content: "\e4ba"; }`, and `e4ba` is the PUA codepoint to insert.

## If you need SVGs instead

Phosphor's source repo ships every icon as an individual SVG — `npm i @phosphor-icons/core`, then `assets/regular/*.svg`. Better than the font for anything that needs per-path color or animation. The mockup doesn't, which is why it uses the font.

MIT — see ../LICENSE.txt.
