# ASSETS.md — vinyl

Rules for anything entering this repo that you did not write. Read alongside
`CLAUDE.md` and `PLAN.md`.

The short version: **all geometry is hand-authored SVG. The only third-party assets
are fonts and, if needed, one or two textures.** Everything else is drawn.

---

## What must be built by hand — never sourced

Do not look for an asset or a code snippet for any of these. They are all circles,
arcs, and rounded rectangles, and they must be parametric because they animate and
respond to live state.

- The plinth, platter, mat, and platter rim
- Grooves (concentric circles with varied spacing)
- The tonearm, pivot, counterweight, headshell, stylus, and its cast shadow
- The specular highlight sweeping the vinyl
- All procedural record label archetypes
- Every animation, including the needle drop

A downloaded PNG or layered PSD of a turntable is a **downgrade**, not a shortcut: the
arm must rotate about its own pivot, the label must be swapped for arbitrary album art
at runtime, and the whole widget must stay crisp at 100%, 150%, and 200% Windows
display scaling. Raster assets defeat all three.

If you find yourself wanting a turntable image, use it as **reference only** — look at
it, then write the SVG.

---

## Fonts

### Choices, pinned

| Role | Family | License | Notes |
|---|---|---|---|
| Label lettering | **Archivo Narrow** | OFL 1.1 | Set in caps with `letter-spacing: 0.14em`. Weights 500 and 600 only. |
| Label alternate | **Bebas Neue** | OFL 1.1 | Use only if Archivo Narrow lacks punch on the arc. Caps-only by design. |
| Timecode | **JetBrains Mono** | OFL 1.1 | Monospaced, so figures are tabular by default and digits never jitter. |

No third face. If you believe one is needed, stop and ask.

### Bundling rules

**Self-host. Do not link `fonts.googleapis.com` or any CDN at runtime.** This project
makes zero network requests once installed (`CLAUDE.md` non-goals), and a font that
fails to load offline breaks the label entirely.

- Download the `.woff2` files at build-setup time and commit them to `src/assets/fonts/`.
- Subset to Latin only using `pyftsubset` or `glyphhanger`. Full families are wasteful
  for a widget that renders a few dozen glyphs.
- Declare with `@font-face` and `font-display: block` — a swap mid-rotation is visible.
- Commit each family's `OFL.txt` alongside the font files. OFL requires the license
  travel with the font.
- Do not rename the font files in a way that obscures the family, and do not modify
  the glyphs (OFL has conditions on derivatives and reserved font names).

### Fallback stacks

```css
--font-label: "Archivo Narrow", "Arial Narrow", system-ui, sans-serif;
--font-mono:  "JetBrains Mono", ui-monospace, Consolas, monospace;
```

---

## Textures

### Try procedural first

Vinyl grain and brushed-metal grain are both cheaper to generate than to source, and
they scale to any DPI for free. Attempt this before downloading anything.

Vinyl surface grain:

```xml
<filter id="grain">
  <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="3" result="n"/>
  <feColorMatrix in="n" type="saturate" values="0"/>
  <feComponentTransfer><feFuncA type="linear" slope="0.06"/></feComponentTransfer>
</filter>
```

Brushed aluminium is directional, so stretch the noise along one axis with
`baseFrequency="0.02 1.4"` and composite at low opacity over the flat steel fill.

**Performance caveat:** an SVG filter re-evaluated every frame on a rotating group is
expensive and will blow the sub-1% idle CPU budget in `CLAUDE.md`. Two options, in
order of preference:

1. Apply grain to a **static** overlay layer that does not rotate, sitting above the
   platter at low opacity. The eye reads it as surface texture regardless.
2. Render the filter once to a data-URI PNG at build time and use that as a `<pattern>`
   fill. No runtime filter cost at all.

Do not apply a live `filter` to the spinning group.

### If a sourced texture is genuinely needed

Only these two sources, both CC0:

- **ambientCG** — `https://ambientcg.com`
- **Poly Haven** — `https://polyhaven.com`

Then: resize to the smallest size that still looks right, convert to WebP or optimized
PNG, keep the file under 100 KB, and record it in `assets/SOURCES.md` (format below).

---

## Icons

Only four glyphs are needed: play, pause, skip-forward, skip-back.

Take them from **Tabler Icons** (MIT) or **Lucide** (ISC). Both are permissive and both
are fine to ship.

Copy the four `<path>` definitions inline into the SVG. Do **not** add an icon library
as a dependency for four paths, and do not bundle a webfont for them.

Add the upstream MIT or ISC notice to `THIRD-PARTY-LICENSES.md` even though only four
paths are used. It is three lines and it is the condition of the license.

---

## Album art

Album art arrives from SMTC at runtime and belongs to whoever owns the recording.

- Display it, transiently, as the record label. That is a normal media-player function.
- Cache it **in memory only**, keyed by `art_id`, and drop the cache on exit.
- Never write it to a persistent location, never build a library of it, never ship any
  of it in the repo or the installer.
- No album art in the README, the GIF, or any screenshot. Demo with the procedural
  labels instead — which conveniently also shows off the better feature.

This last rule matters: a README GIF full of real album covers is the one place this
project could plausibly draw a complaint.

---

## Licenses

### Allowed

| License | Obligation |
|---|---|
| CC0 / public domain | None |
| MIT, ISC, BSD-2/3 | Reproduce the notice |
| Apache-2.0 | Reproduce notice and `NOTICE` file if present |
| SIL OFL 1.1 | Ship `OFL.txt` with the font; respect reserved names |
| CC-BY 4.0 | Attribute in `THIRD-PARTY-LICENSES.md` and the README |

### Not allowed — do not download these, and stop and ask if one seems necessary

- **CC-BY-NC** anything. This is a public release; non-commercial terms are a trap even
  though the project is free.
- **CC-BY-ND**. No derivatives means you cannot resize or recolour it.
- **CC-BY-SA**. Copyleft on an asset creates obligations across the project. Avoid.
- **"Free for personal use."** Not usable in a public GitHub release.
- **Freepik, Flaticon, Vecteezy free tiers, Envato, Adobe Stock.** Attribution and
  redistribution terms that don't fit a bundled desktop binary.
- Anything from Dribbble, Behance, or Pinterest. These are portfolios, not asset
  libraries.
- **CodePen**. There is no blanket open license; a pen is the author's copyright unless
  they explicitly state otherwise. "It was public" is not permission.
- **Stack Overflow code, copied verbatim.** Answers are CC-BY-SA, which is copyleft.
  Read answers to understand an API, then write your own implementation. This applies
  especially to the WinRT and WASAPI code, where SO will be a heavy reference.
- Any file whose origin you cannot name.

### Rust dependencies

Run `cargo deny check licenses` in CI with an allowlist matching the table above.
Generate attributions with `cargo about`. A permissive Rust crate still needs its
notice reproduced in the shipped binary's license file.

---

## Bookkeeping

Two files, kept current as assets land. An asset without an entry is a bug.

**`assets/SOURCES.md`** — one row per non-authored file:

```
| File | Source URL | License | Retrieved | Modifications |
|---|---|---|---|---|
| fonts/ArchivoNarrow.woff2 | fonts.google.com/specimen/Archivo+Narrow | OFL 1.1 | 2026-07-25 | Subset to Latin |
```

**`THIRD-PARTY-LICENSES.md`** — full license text for every dependency and asset,
generated for Rust crates and hand-maintained for fonts, icons, and textures. Ship it
inside the installer, not just in the repo.

### Gate check

Before the Phase 7 release, verify:

- Every file under `src/assets/` has a `SOURCES.md` row.
- Every row's license appears in the allowed table.
- `cargo deny check licenses` passes.
- No album art anywhere in the repo, the installer, or the README.
- `THIRD-PARTY-LICENSES.md` is included in the built installer.
