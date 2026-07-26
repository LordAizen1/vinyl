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
  { field: "#2b2f35", ink: "#e8e9eb", accent: "#6e1f26", sub: "#8a8f98" },
  { field: "#1f2a2e", ink: "#dbe2e4", accent: "#2f6f6b", sub: "#7e8f92" },
  { field: "#2c2a25", ink: "#e4e2de", accent: "#b8792c", sub: "#948b7a" },
  { field: "#242835", ink: "#dde2f0", accent: "#4a5a8a", sub: "#838aa0" },
  { field: "#2a2521", ink: "#e2ddd8", accent: "#7d4a2e", sub: "#95897c" },
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

/** A plausible catalogue number, derived from the same hash. */
function catalogueOf(seed) {
  const prefix = ["VNL", "OXB", "HFI", "STL", "LMP"][(seed >>> 3) % 5];
  const number = 1000 + ((seed >>> 7) % 9000);
  const side = (seed >>> 17) % 2 === 0 ? "A" : "B";
  return `${prefix} ${number} · ${side}`;
}

/* Arc paths. sweep-flag 1 in a y-down system gives the upper arc; sweep-flag 0
   gives the lower one, traversed left to right so the type reads correctly. */
function arcs() {
  return `
    <defs>
      <path id="lblTop" d="M ${CX - 16} ${CY} A 16 16 0 0 1 ${CX + 16} ${CY}" fill="none"/>
      <path id="lblBottom" d="M ${CX - 19.5} ${CY} A 19.5 19.5 0 0 0 ${CX + 19.5} ${CY}" fill="none"/>
      <path id="lblTopWide" d="M ${CX - 20} ${CY} A 20 20 0 0 1 ${CX + 20} ${CY}" fill="none"/>
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
    <circle cx="${CX}" cy="${CY}" r="${R - 0.7}" fill="none" stroke="${p.sub}" stroke-opacity=".32" stroke-width=".5"/>
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="none" stroke="#000" stroke-opacity=".55" stroke-width=".9"/>`;
}

function arced(pathId, text, size, weight, fill, spacing) {
  return `<text class="lbl-fit" data-arc="${pathId}" font-family="var(--font-label)" font-size="${size}"
      font-weight="${weight}" letter-spacing="${spacing}" fill="${fill}" text-anchor="middle"
    ><textPath href="#${pathId}" startOffset="50%">${escapeXml(text)}</textPath></text>`;
}

function straight(x, y, text, size, weight, fill, spacing) {
  return `<text class="lbl-fit" data-width="40" x="${x}" y="${y}" font-family="var(--font-label)" font-size="${size}"
      font-weight="${weight}" letter-spacing="${spacing}" fill="${fill}" text-anchor="middle"
    >${escapeXml(text)}</text>`;
}

function catalogue(y, text, fill) {
  return `<text x="${CX}" y="${y}" font-family="var(--font-mono)" font-size="3" fill="${fill}"
      fill-opacity=".72" text-anchor="middle">${escapeXml(text)}</text>`;
}

/* ─────────────────────────── archetypes ─────────────────────────── */

/** Concentric rings with a single accent band. The quiet one. */
function rings(p, title, artist, cat) {
  return `
    ${field(p)}
    <circle cx="${CX}" cy="${CY}" r="21" fill="none" stroke="${p.accent}" stroke-width="1.1"/>
    <circle cx="${CX}" cy="${CY}" r="19" fill="none" stroke="#000" stroke-opacity=".22" stroke-width=".4"/>
    ${arced("lblTop", title, 4.8, 600, p.ink, 0.55)}
    ${artist ? straight(CX, CY + 10, artist, 4, 500, p.sub, 0.45) : ""}
    ${catalogue(CY + 16.5, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** A band above the spindle. Blue Note, roughly.
 *
 *  The band sits in the upper half rather than across the middle: centred, the
 *  title collides with the spindle hole, which no real label does. */
function band(p, title, artist, cat) {
  return `
    ${field(p)}
    <path d="M ${CX - 25.6} ${CY - 14.5} h 51.2 v 12.2 h -51.2 z" fill="${p.accent}" opacity=".92"
          clip-path="url(#cLab)"/>
    ${straight(CX, CY - 5.6, title, 4.6, 600, "#f4f5f6", 0.5)}
    ${artist ? straight(CX, CY + 11, artist, 3.8, 500, p.sub, 0.42) : ""}
    ${catalogue(CY + 17.5, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** Accent field with the type reversed out. The loud one. */
function solid(p, title, artist, cat) {
  return `
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="${p.accent}"/>
    <circle cx="${CX}" cy="${CY}" r="${R}" fill="url(#gLabShade)"/>
    <circle cx="${CX}" cy="${CY}" r="22.5" fill="none" stroke="${p.field}" stroke-opacity=".55" stroke-width=".6"/>
    ${arced("lblTopWide", title, 4.6, 600, "#f4f5f6", 0.6)}
    ${artist ? straight(CX, CY + 11, artist, 4, 500, "rgba(255,255,255,.72)", 0.45) : ""}
    ${catalogue(CY + 17, cat, "rgba(255,255,255,.6)")}
    ${rim(p)}${spindle}`;
}

/** Title arced above, artist arced below. Symmetrical and formal. */
function halo(p, title, artist, cat) {
  return `
    ${field(p)}
    <circle cx="${CX}" cy="${CY}" r="23" fill="none" stroke="${p.accent}" stroke-width=".8"/>
    <circle cx="${CX}" cy="${CY}" r="12.5" fill="none" stroke="${p.accent}" stroke-opacity=".5" stroke-width=".6"/>
    ${arced("lblTop", title, 4.6, 600, p.ink, 0.5)}
    ${artist ? arced("lblBottom", artist, 3.8, 500, p.sub, 0.45) : ""}
    ${catalogue(CY + 7.5, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

/** A quarter arc in the outer band. The asymmetric one, and the one that most
 *  obviously reads as turning.
 *
 *  Drawn as a stroked arc rather than a filled wedge so it stays out of the
 *  middle of the label, where a solid wedge fights the type for contrast. */
function quadrant(p, title, artist, cat) {
  const ringR = 22.6;
  const quarter = (2 * Math.PI * ringR) / 4;
  return `
    ${field(p)}
    <circle cx="${CX}" cy="${CY}" r="${ringR}" fill="none" stroke="${p.accent}" stroke-width="5.4"
            stroke-dasharray="${quarter.toFixed(2)} ${(quarter * 3).toFixed(2)}"
            transform="rotate(-86 ${CX} ${CY})" opacity=".92"/>
    <circle cx="${CX}" cy="${CY}" r="19.2" fill="none" stroke="#000" stroke-opacity=".28" stroke-width=".5"/>
    ${straight(CX, CY - 7.5, title, 4.4, 600, p.ink, 0.5)}
    ${artist ? straight(CX, CY + 10, artist, 3.8, 500, p.sub, 0.42) : ""}
    ${catalogue(CY + 16, cat, p.sub)}
    ${rim(p)}${spindle}`;
}

const ARCHETYPES = [rings, band, solid, halo, quadrant];

/**
 * Builds the label markup for a track.
 *
 * @param {{title?: string, artist?: string, sourceApp?: string}} track
 * @returns {string} SVG markup for the contents of the label group
 */
export function proceduralLabel(track) {
  const title = (track.title || "Untitled").toUpperCase();

  // Deliberately no fall back to the source app. WhatsApp desktop hosts as the
  // shared msedgewebview2.exe, so that would print a process name on the
  // record. When there is no artist the line is simply omitted; the panel
  // already shows the source. See docs/FINDINGS.md.
  const artist = (track.artist || "").toUpperCase();

  // Source app in the seed: without it, every blank-metadata source collapses
  // onto one label. See docs/FINDINGS.md.
  const seed = hashOf(`${track.artist || ""}|${track.title || ""}|${track.sourceApp || ""}`);

  const palette = PALETTES[seed % PALETTES.length];
  const archetype = ARCHETYPES[(seed >>> 11) % ARCHETYPES.length];

  return arcs() + archetype(palette, title, artist, catalogueOf(seed));
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
 * Truncates any label text that would overrun its arc or its width.
 *
 * Must run after the markup is in the document, because it measures rendered
 * glyphs. Phase 0 found artist strings like "The Weeknd — After Hours
 * (Deluxe)", which comfortably overrun a 26px label.
 */
export function fitLabelText(root) {
  for (const el of root.querySelectorAll(".lbl-fit")) {
    const arcId = el.dataset.arc;
    const limit = arcId
      ? arcLengthOf(root, arcId) * 0.94
      : Number(el.dataset.width || 40);

    const full = el.textContent;
    if (el.getComputedTextLength() <= limit) continue;

    for (let cut = full.length - 1; cut > 0; cut -= 1) {
      el.textContent = `${full.slice(0, cut).trimEnd()}…`;
      if (el.getComputedTextLength() <= limit) break;
    }
  }
}

function arcLengthOf(root, id) {
  const path = root.querySelector(`#${id}`);
  return path ? path.getTotalLength() : 40;
}
