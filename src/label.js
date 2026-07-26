/**
 * Procedural record labels.
 *
 * CLAUDE.md constraint 3: album art is often missing, and a blank label looks
 * broken rather than minimal. So a missing thumbnail produces a designed label
 * instead of a hole. This is a feature, not a fallback.
 *
 * The seed includes the source app, not just artist and title. Phase 0 found
 * that Windows Photos, Netflix and WhatsApp all report blank or generic
 * metadata, so seeding on `artist + title` alone would give every one of them
 * the identical label. See docs/FINDINGS.md.
 *
 * Same input always yields the same label.
 *
 * Built around a monogram rather than a block of text. The label renders about
 * 52px across, where set type turns to mush; a large letterform still reads as
 * a deliberate mark at that size, which is how real labels survive being small.
 */

/* Label geometry, in the deck's 232x220 viewBox. */
const CX = 96;
const CY = 112;
const R = 26;

/**
 * Five palettes, all dark-field with light ink so they sit inside the hi-fi
 * register rather than drifting to the cream-and-terracotta look CLAUDE.md
 * rules out.
 */
const PALETTES = [
  { field: "#2b2f35", ink: "#eceef0", accent: "#8d2731", sub: "#9aa0a8" },
  { field: "#1c282c", ink: "#e2eaec", accent: "#2f8079", sub: "#8ba0a2" },
  { field: "#2e2b24", ink: "#efece5", accent: "#c4862f", sub: "#a89c88" },
  { field: "#232839", ink: "#e4e9f5", accent: "#5169a6", sub: "#939bb2" },
  { field: "#2c2621", ink: "#efe7dd", accent: "#96562f", sub: "#a5968a" },
];

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
 * no artist, or a record glyph when there is neither.
 *
 * Takes the first letter-or-digit rather than the first character, so a title
 * opening with a bracket or a quote does not produce a punctuation monogram.
 */
function monogramOf(track) {
  const source = (track.artist || track.title || "").trim();
  for (const char of source) {
    if (/\p{L}|\p{N}/u.test(char)) return char.toUpperCase();
  }
  return "♪"; // an eighth note, for a source that tells us nothing at all
}

/** A plausible catalogue number, derived from the same hash. */
function catalogueOf(seed) {
  const prefix = ["VNL", "OXB", "HFI", "STL", "LMP"][(seed >>> 3) % 5];
  return `${prefix} ${1000 + ((seed >>> 7) % 9000)}`;
}

/* Arc paths. sweep-flag 1 in a y-down system gives the upper arc; sweep-flag 0
   gives the lower one, traversed left to right so the type reads correctly. */
function arcs() {
  return `
    <defs>
      <path id="lblTop" d="M ${CX - 18.5} ${CY} A 18.5 18.5 0 0 1 ${CX + 18.5} ${CY}" fill="none"/>
      <path id="lblTopWide" d="M ${CX - 21} ${CY} A 21 21 0 0 1 ${CX + 21} ${CY}" fill="none"/>
      <path id="lblBottom" d="M ${CX - 19.5} ${CY} A 19.5 19.5 0 0 0 ${CX + 19.5} ${CY}" fill="none"/>
    </defs>`;
}

const spindle = `
  <circle cx="${CX}" cy="${CY}" r="2.6" fill="#0b0c0d"/>
  <circle cx="${CX}" cy="${CY}" r="2.6" fill="none" stroke="rgba(0,0,0,.55)" stroke-width=".4"/>`;

function field(p) {
  return `
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="${p.field}"/>
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="url(#gLabShade)"/>`;
}

function rim(p) {
  return `
    <circle cx="${CX}" cy="${CY}" r="${R - 0.7}" fill="none" stroke="${p.sub}" stroke-opacity=".34" stroke-width=".5"/>
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="none" stroke="#000" stroke-opacity=".55" stroke-width=".9"/>`;
}

/**
 * The monogram itself.
 *
 * Sits at CY-9 so its descender clears the spindle hole at CY. Centred on the
 * label it looked like the hole had been punched through the letterform.
 */
function mark(letter, fill, size = 18) {
  return `<text x="${CX}" y="${CY - 9}" font-family="var(--font-display)" font-size="${size}"
      font-weight="700" fill="${fill}" text-anchor="middle" dominant-baseline="middle"
    >${escapeXml(letter)}</text>`;
}

function arced(pathId, text, size, fill, spacing, weight = 500) {
  return `<text class="lbl-fit" data-arc="${pathId}" font-family="var(--font-display)" font-size="${size}"
      font-weight="${weight}" letter-spacing="${spacing}" fill="${fill}" text-anchor="middle"
    ><textPath href="#${pathId}" startOffset="50%">${escapeXml(text)}</textPath></text>`;
}

function catalogue(y, text, fill) {
  return `<text x="${CX}" y="${y}" font-family="var(--font-mono)" font-size="2.9" fill="${fill}"
      fill-opacity=".7" text-anchor="middle">${escapeXml(text)}</text>`;
}

/* ─────────────────────────── archetypes ───────────────────────────
   Each is one monogram, one arced line, and at most one ring device.

   Vertical budget, so nothing collides: monogram CY-15 to CY-3, spindle hole
   CY±2.6, top arc around CY-19, bottom arc around CY+19. An archetype using
   the bottom arc puts its catalogue number at CY+10, between the hole and the
   arc; one using the top arc puts it at CY+20, below everything. */

/** A ruled circle around the mark. The quiet one. */
function ringed(p, mono, title, cat) {
  return `
    ${field(p)}
    <circle cx="${CX}" cy="${CY}" r="17" fill="none" stroke="${p.accent}" stroke-width="1"/>
    ${mark(mono, p.ink)}
    ${arced("lblTopWide", title, 3.6, p.sub, 0.35)}
    ${catalogue(CY + 20, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** The mark reversed out of a solid accent disc. The loud one. */
function disc(p, mono, title, cat) {
  return `
    ${field(p)}
    <circle cx="${CX}" cy="${CY - 3}" r="17" fill="${p.accent}"/>
    ${mark(mono, "#f6f4f1")}
    ${arced("lblTopWide", title, 3.6, p.sub, 0.35)}
    ${catalogue(CY + 20, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** A band behind the mark, Blue Note by way of a monogram. */
function banded(p, mono, title, cat) {
  return `
    ${field(p)}
    <path d="M ${CX - 25.8} ${CY - 17.5} h 51.6 v 16.5 h -51.6 z" fill="${p.accent}"
          opacity=".92" clip-path="url(#cLab)"/>
    ${mark(mono, "#f6f4f1")}
    ${arced("lblBottom", title, 3.5, p.sub, 0.35)}
    ${catalogue(CY + 10, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** A quarter arc in the outer band. The asymmetric one, and the one that most
    obviously reads as turning. */
function quartered(p, mono, title, cat) {
  const ringR = 22.6;
  const quarter = (2 * Math.PI * ringR) / 4;
  return `
    ${field(p)}
    <circle cx="${CX}" cy="${CY}" r="${ringR}" fill="none" stroke="${p.accent}" stroke-width="5"
            stroke-dasharray="${quarter.toFixed(2)} ${(quarter * 3).toFixed(2)}"
            transform="rotate(-86 ${CX} ${CY})" opacity=".92"/>
    ${mark(mono, p.ink)}
    ${arced("lblBottom", title, 3.5, p.sub, 0.35)}
    ${catalogue(CY + 10, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** Hairline rules above and below the mark. The formal one. */
function ruled(p, mono, title, cat) {
  return `
    ${field(p)}
    <path d="M ${CX - 14} ${CY - 18} h 28 M ${CX - 14} ${CY + 5.5} h 28"
          stroke="${p.accent}" stroke-width="1.1"/>
    <path d="M ${CX - 14} ${CY - 16.2} h 28 M ${CX - 14} ${CY + 7.3} h 28"
          stroke="${p.accent}" stroke-opacity=".45" stroke-width=".5"/>
    ${mark(mono, p.ink)}
    ${arced("lblBottom", title, 3.5, p.sub, 0.35)}
    ${catalogue(CY + 13, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

const ARCHETYPES = [ringed, disc, banded, quartered, ruled];

/**
 * Builds the label markup for a track.
 *
 * @param {{title?: string, artist?: string, sourceApp?: string}} track
 * @returns {string} SVG markup for the contents of the label group
 */
export function proceduralLabel(track) {
  // The arced line carries the artist when we have one, since the panel
  // already shows the title in full a few pixels away. When there is no
  // artist it falls back to the title rather than repeating the source app,
  // which would put a process name on the record. See docs/FINDINGS.md.
  const line = (track.artist || track.title || "").toUpperCase();

  const seed = hashOf(
    `${track.artist || ""}|${track.title || ""}|${track.sourceApp || ""}`,
  );

  const palette = PALETTES[seed % PALETTES.length];
  const archetype = ARCHETYPES[(seed >>> 11) % ARCHETYPES.length];

  return (
    arcs() + archetype(palette, monogramOf(track), line, catalogueOf(seed))
  );
}

/** The real-artwork label: the art itself, ringed like a pressed label. */
export function artLabel(url) {
  return `
    <g clip-path="url(#cLab)">
      <image href="${escapeXml(url)}" x="${CX - R}" y="${CY - R}" width="${R * 2}" height="${R * 2}"
             preserveAspectRatio="xMidYMid slice"/>
      <circle cx="${CX}" cy="${CY}" r="${R}" fill="url(#gLabShade)"/>
    </g>
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="none" stroke="#000" stroke-opacity=".6" stroke-width=".9"/>
    <circle cx="${CX}" cy="${CY}" r="${R - 0.7}" fill="none" stroke="#8a8f98" stroke-opacity=".22" stroke-width=".5"/>
    ${spindle}`;
}

/**
 * Truncates any label text that would overrun its arc.
 *
 * Must run after the markup is in the document, because it measures rendered
 * glyphs. Phase 0 found artist strings like "The Weeknd — After Hours
 * (Deluxe)", which comfortably overrun a 26px label.
 */
export function fitLabelText(root) {
  for (const el of root.querySelectorAll(".lbl-fit")) {
    const arcId = el.dataset.arc;
    const limit = arcId
      ? arcLengthOf(root, arcId) * 0.92
      : Number(el.dataset.width || 40);

    // Write to the textPath, not to the <text>. Setting textContent on the
    // parent replaces the textPath child with a bare text node, which silently
    // drops the string off the arc and renders it at the origin instead.
    // SVGTextPathElement extends SVGTextContentElement, so it measures itself.
    const target = el.querySelector("textPath") ?? el;

    const full = target.textContent;
    if (target.getComputedTextLength() <= limit) continue;

    for (let cut = full.length - 1; cut > 0; cut -= 1) {
      target.textContent = `${full.slice(0, cut).trimEnd()}…`;
      if (target.getComputedTextLength() <= limit) break;
    }
  }
}

function arcLengthOf(root, id) {
  const path = root.querySelector(`#${id}`);
  return path ? path.getTotalLength() : 40;
}
