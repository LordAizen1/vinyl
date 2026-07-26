# SOURCES.md — every file in this repo that was not authored here

Rules live in `ASSETS.md`. An asset without a row here is a bug.

All turntable geometry is hand-authored SVG and therefore does not appear below.
Vinyl and brushed-metal grain are generated with `feTurbulence`, not sourced.

| File | Source URL | License | Retrieved | Modifications |
|---|---|---|---|---|
| src/assets/fonts/ArchivoNarrow-latin.woff2 | fonts.google.com/specimen/Archivo+Narrow | OFL 1.1 | 2026-07-26 | None. Latin subset as served by Google Fonts. Variable weight axis, covers 500 and 600. |
| src/assets/fonts/JetBrainsMono-latin.woff2 | fonts.google.com/specimen/JetBrains+Mono | OFL 1.1 | 2026-07-26 | None. Latin subset as served by Google Fonts. |
| src/assets/fonts/PlayfairDisplay-latin.woff2 | fonts.google.com/specimen/Playfair+Display | OFL 1.1 | 2026-07-26 | None. Latin subset as served by Google Fonts. Variable weight axis, 400 to 900. |
| src/assets/fonts/PlayfairDisplay-OFL.txt | github.com/google/fonts/blob/main/ofl/playfairdisplay/OFL.txt | OFL 1.1 | 2026-07-26 | None |
| src/assets/fonts/ArchivoNarrow-OFL.txt | github.com/google/fonts/blob/main/ofl/archivonarrow/OFL.txt | OFL 1.1 | 2026-07-26 | None |
| src/assets/fonts/JetBrainsMono-OFL.txt | github.com/google/fonts/blob/main/ofl/jetbrainsmono/OFL.txt | OFL 1.1 | 2026-07-26 | None |

## Notes

The two woff2 files are the Latin subsets Google Fonts serves, which is why they are
already small (18.3 KB and 20.7 KB). `ASSETS.md` asks for a `pyftsubset` or `glyphhanger`
pass; that is only worth doing if the widget's glyph coverage turns out to be much
narrower than Latin, and it should be settled before the Phase 7 release rather than now.

Archivo Narrow ships as a variable font on the weight axis. Google serves the identical
file for weights 500 and 600, so there is one file and the `@font-face` declares a
`font-weight` range rather than two separate faces.

Neither file is renamed in a way that obscures its family, and neither has modified
glyphs, so the OFL reserved font name conditions are not engaged.
