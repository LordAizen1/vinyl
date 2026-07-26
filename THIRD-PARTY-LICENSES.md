# Third-party licenses

Required by `ASSETS.md`. Ships inside the installer, not only in the repo.

Turntable geometry, the procedural labels and every animation are hand-authored
and appear nowhere below.

---

## Fonts

### Archivo Narrow

SIL Open Font License 1.1. Full text: `src/assets/fonts/ArchivoNarrow-OFL.txt`.

### JetBrains Mono

SIL Open Font License 1.1. Full text: `src/assets/fonts/JetBrainsMono-OFL.txt`.

### Pirata One

SIL Open Font License 1.1. Full text: `src/assets/fonts/PirataOne-OFL.txt`.

None of the three has been modified, renamed to obscure its family, or had its
glyphs altered, so the OFL reserved font name conditions are not engaged.

---

## Icons

The transport glyphs (shuffle, skip back, skip forward, repeat) and the music
note in the display are adapted from **Lucide**, drawn inline as SVG paths
rather than pulled in as a dependency.

Lucide is ISC licensed, and derives in part from Feather, which is MIT.

### Lucide — ISC License

```
ISC License

Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of
Feather (MIT). All other copyright (c) for Lucide are held by Lucide Contributors
2022.

Permission to use, copy, modify, and/or distribute this software for any purpose
with or without fee is hereby granted, provided that the above copyright notice
and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH
REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER
TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF
THIS SOFTWARE.
```

The play and pause glyphs are hand-drawn and not from any icon set.

---

## Rust dependencies

Not yet generated. `ASSETS.md` requires `cargo about` output and a
`cargo deny check licenses` pass before the Phase 7 release; both are still to
be wired up.
