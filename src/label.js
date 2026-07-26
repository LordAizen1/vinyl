/**
 * Procedural record labels.
 *
 * CLAUDE.md constraint 3: album art is often missing, and a blank label looks
 * broken rather than minimal. So a missing thumbnail produces a designed label
 * instead of a hole. This is a feature, not a fallback.
 *
 * The look is the iridescent mother-of-pearl label from the reference design:
 * a seeded fractal-noise swirl, with the artist's initial reversed out of it.
 * The seed includes the source app, not just artist and title, because Phase 0
 * found that Windows Photos, Netflix and WhatsApp all report blank or generic
 * metadata; seeding on artist and title alone would give every one of them the
 * identical label. See docs/FINDINGS.md.
 *
 * Same input always yields the same label.
 */

/* Label geometry, in the deck's 232x232 viewBox. */
const CX = 335;
const CY = 345;
const R = 92;

/**
 * Abalone palettes as [stop, r, g, b]. Teal and green through to violet, which
 * is what real paua shell does and what the reference shows.
 */
const PALETTES = [
  [
    [0, 15, 63, 58],
    [0.22, 31, 119, 102],
    [0.42, 83, 184, 151],
    [0.58, 205, 240, 221],
    [0.72, 143, 180, 214],
    [0.86, 154, 111, 184],
    [1, 217, 169, 214],
  ],
  [
    [0, 40, 26, 66],
    [0.25, 84, 52, 128],
    [0.45, 158, 105, 199],
    [0.6, 236, 214, 244],
    [0.74, 120, 144, 214],
    [0.88, 72, 178, 190],
    [1, 190, 240, 235],
  ],
  [
    [0, 7, 44, 66],
    [0.24, 14, 90, 118],
    [0.46, 46, 158, 164],
    [0.62, 196, 235, 226],
    [0.76, 120, 196, 164],
    [0.9, 206, 182, 120],
    [1, 240, 226, 190],
  ],
  [
    [0, 48, 20, 34],
    [0.24, 104, 42, 74],
    [0.46, 176, 92, 120],
    [0.62, 244, 214, 214],
    [0.78, 150, 168, 206],
    [1, 96, 196, 190],
  ],
];

/**
 * The hue this track's procedural label is built around, so the screen can be
 * tinted to match even when there is no cover art to sample.
 *
 * @returns {number} hue in degrees
 */
export function proceduralHue(track) {
  const seed = hashOf(
    `${track.artist || ""}|${track.title || ""}|${track.sourceApp || ""}`,
  );
  const palette = PALETTES[seed % PALETTES.length];
  // The stop nearest the middle carries the palette's character; the ends are
  // the near-black and near-white extremes.
  const mid = palette[Math.floor(palette.length / 2)];
  return rgbToHue(mid[1], mid[2], mid[3]);
}

function rgbToHue(r, g, b) {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  if (max === min) return 0;
  const d = max - min;
  let h;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  h *= 60;
  return h < 0 ? h + 360 : h;
}

/** FNV-1a, so a given track always presses the same label. */
export function hashOf(input) {
  let hash = 2166136261;
  for (let i = 0; i < input.length; i += 1) {
    hash ^= input.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function escapeXml(value) {
  return String(value).replace(
    /[<>&"]/g,
    (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;" })[c],
  );
}

/**
 * The monogram: the first letter of the artist, or of the title when there is
 * no artist, or a note glyph when there is neither.
 *
 * Takes the first letter-or-digit rather than the first character, so a title
 * opening with a bracket or a quote does not produce a punctuation monogram.
 */
function monogramOf(track) {
  const source = (track.artist || track.title || "").trim();
  for (const char of source) {
    if (/\p{L}|\p{N}/u.test(char)) return char.toUpperCase();
  }
  return "♪";
}

/** A plausible catalogue number, derived from the same hash. */
function catalogueOf(seed) {
  const prefix = ["VNL", "OXB", "HFI", "STL", "LMP"][(seed >>> 3) % 5];
  return `${prefix} ${1000 + ((seed >>> 7) % 9000)}`;
}

/* ═══════════════════════════ abalone ═══════════════════════════
 *
 * Generated from seeded fractal noise with domain warping, which is what turns
 * smooth noise into the banded, shell-like swirl.
 *
 * Rendered once per track and cached. It is the one genuinely expensive thing
 * in the frontend, so the texture is deliberately small (128px, upscaled by the
 * SVG) with a low octave count. That is affordable on a track change and would
 * not be on a timer.
 */
const ABALONE_SIZE = 128;
const abaloneCache = new Map();

/** Small, fast PRNG. Same seed, same swirl. */
function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function abaloneTexture(seed) {
  const key = String(seed);
  const cached = abaloneCache.get(key);
  if (cached) return cached;

  const palette = PALETTES[seed % PALETTES.length];
  const size = ABALONE_SIZE;

  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d");
  const image = ctx.createImageData(size, size);
  const data = image.data;

  // Value-noise lattice, shuffled by the seed.
  const rng = mulberry32(seed);
  const perm = new Uint8Array(512);
  const order = new Uint8Array(256);
  for (let i = 0; i < 256; i += 1) order[i] = i;
  for (let i = 255; i > 0; i -= 1) {
    const j = Math.floor(rng() * (i + 1));
    const tmp = order[i];
    order[i] = order[j];
    order[j] = tmp;
  }
  for (let i = 0; i < 512; i += 1) perm[i] = order[i & 255];

  const values = new Float32Array(256);
  for (let i = 0; i < 256; i += 1) values[i] = rng();

  const lattice = (ix, iy) => values[perm[(ix + perm[iy & 255]) & 255]];
  const fade = (t) => t * t * t * (t * (t * 6 - 15) + 10);

  function noise(x, y) {
    const ix = Math.floor(x);
    const iy = Math.floor(y);
    const fx = x - ix;
    const fy = y - iy;
    const a = lattice(ix, iy);
    const b = lattice(ix + 1, iy);
    const c = lattice(ix, iy + 1);
    const d = lattice(ix + 1, iy + 1);
    const u = fade(fx);
    const v = fade(fy);
    return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
  }

  function fbm(x, y, octaves) {
    let sum = 0;
    let amp = 0.5;
    let freq = 1;
    for (let k = 0; k < octaves; k += 1) {
      sum += amp * noise(x * freq, y * freq);
      amp *= 0.5;
      freq *= 2.03;
    }
    return sum / (1 - 0.5 ** octaves);
  }

  function sample(t) {
    const clamped = Math.min(1, Math.max(0, t));
    for (let k = 1; k < palette.length; k += 1) {
      if (clamped <= palette[k][0]) {
        const lo = palette[k - 1];
        const hi = palette[k];
        const u = (clamped - lo[0]) / (hi[0] - lo[0] || 1);
        return [
          lo[1] + (hi[1] - lo[1]) * u,
          lo[2] + (hi[2] - lo[2]) * u,
          lo[3] + (hi[3] - lo[3]) * u,
        ];
      }
    }
    const last = palette[palette.length - 1];
    return [last[1], last[2], last[3]];
  }

  let offset = 0;
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const nx = x / size;
      const ny = y / size;
      // Domain warping: this is what turns smooth noise into shell banding.
      const wx = fbm(nx * 3 + 11.7, ny * 3 + 4.2, 3);
      const wy = fbm(nx * 3 + 5.2, ny * 3 + 1.3, 3);
      let t = fbm(nx * 2.4 + 2.4 * wx, ny * 2.4 + 2.4 * wy, 3);
      const band = 0.5 + 0.5 * Math.sin((nx * 3.6 + wx * 3.2) * 5.6 + t * 7);
      t = t * 0.55 + band * 0.45;

      const shimmer = 0.82 + 0.36 * fbm(nx * 9 + 8.8, ny * 9 + 3.1, 2);
      const rgb = sample(t);
      data[offset] = Math.min(255, rgb[0] * shimmer);
      data[offset + 1] = Math.min(255, rgb[1] * shimmer);
      data[offset + 2] = Math.min(255, rgb[2] * shimmer);
      data[offset + 3] = 255;
      offset += 4;
    }
  }

  ctx.putImageData(image, 0, 0);
  const url = canvas.toDataURL("image/png");
  abaloneCache.set(key, url);
  return url;
}

/* ═══════════════════════════ the labels ═══════════════════════════ */

/* The record's own rings around the label. The spindle itself is drawn by the
   deck, above the rotating group, since it does not turn with the record. */
const rim = `
  <circle cx="${CX}" cy="${CY}" r="${R + 1}" fill="none" stroke="#0b0c0e" stroke-width="3.4"/>
  <circle cx="${CX}" cy="${CY}" r="${R + 3.6}" fill="none" stroke="rgba(255,255,255,.07)" stroke-width="1.6"/>`;

/**
 * Builds the label markup for a track.
 *
 * @param {{title?: string, artist?: string, sourceApp?: string}} track
 * @returns {string} SVG markup for the contents of the label group
 */
export function proceduralLabel(track) {
  const seed = hashOf(
    `${track.artist || ""}|${track.title || ""}|${track.sourceApp || ""}`,
  );

  const mono = monogramOf(track);
  const catalogue = catalogueOf(seed);

  return `
    <defs>
      <filter id="lblInk" x="-30%" y="-30%" width="160%" height="160%">
        <feDropShadow dx="0" dy="2" stdDeviation="2.5" flood-color="#000" flood-opacity="0.45"/>
      </filter>
    </defs>
    <g clip-path="url(#cLab)">
      <image href="${abaloneTexture(seed)}" x="${CX - R}" y="${CY - R}"
             width="${R * 2}" height="${R * 2}" preserveAspectRatio="xMidYMid slice"/>
      <circle cx="${CX}" cy="${CY}" r="${R}" fill="url(#gLabShade)"/>
    </g>
    <text x="${CX}" y="${CY - 26}" font-family="var(--font-display)" font-size="62"
          fill="#ffffff" fill-opacity=".92" text-anchor="middle"
          dominant-baseline="middle" filter="url(#lblInk)">${escapeXml(mono)}</text>
    <text x="${CX}" y="${CY + 52}" font-family="var(--font-mono)" font-size="10"
          fill="#ffffff" fill-opacity=".6" text-anchor="middle"
    >${escapeXml(catalogue)}</text>
    ${rim}`;
}

/** The real-artwork label: the cover itself, ringed like a pressed label. */
export function artLabel(url) {
  return `
    <g clip-path="url(#cLab)">
      <image href="${escapeXml(url)}" x="${CX - R}" y="${CY - R}" width="${R * 2}" height="${R * 2}"
             preserveAspectRatio="xMidYMid slice"/>
    </g>
    ${rim}`;
}

/**
 * Kept for the caller's benefit: the abalone label has no arced type to fit, so
 * there is nothing to measure. Retained so main.js does not need to know which
 * label style is in use.
 */
export function fitLabelText() {}
