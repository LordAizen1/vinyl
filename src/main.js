import {
  artLabel,
  fitLabelText,
  proceduralHue,
  proceduralLabel,
} from "./label.js";

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const RM = matchMedia("(prefers-reduced-motion: reduce)");

const widget = $("widget");
const spinG = $("spinG");
const armG = $("armG");
const labelG = $("labelG");

/* ══════════════════════════════════════════════════════════════════════
 * Geometry, in the deck's 715x700 viewBox.
 *
 * Record centre (335,345) r 303, label r 92, arm pivot (640,105). The arm is
 * drawn pointing along +x with its tip at pivot + 392, so a rotation is the
 * absolute bearing of the stylus from the pivot.
 * ══════════════════════════════════════════════════════════════════════ */
const CENTER = { x: 335, y: 345 };
const PIVOT = { x: 640, y: 105 };
const PIVOT_DIST = Math.hypot(PIVOT.x - CENTER.x, PIVOT.y - CENTER.y);

/**
 * Pivot to contact, and the bearing of the contact in the arm's drawn pose.
 *
 * The tube runs straight to (860,105), bends to (990,128), and the headshell
 * is set there at a 30 degree offset. The contact point is under the front of
 * the head, mid-width: local (36, 24) inside that rotated group, which lands
 * at (1009.18, 166.79). Nothing is drawn there — the stylus points down, so
 * from above it is hidden by the head — but the arm still tracks to it.
 *
 * So the arm is not "pointing along +x": pivot to contact measures 374.31 at
 * 9.5 degrees. Both are derived from the drawn geometry, because assuming zero
 * would leave the cartridge several degrees off the groove it is tracking.
 * Re-measure these whenever the headshell or cartridge is resized.
 */
const ARM_LEN = 374.31;
const DRAWN_ANGLE = 9.5;

/**
 * Where the stylus sits at the start and end of a track.
 *
 * The lead-in is well inside the record's 303 rather than at the outermost
 * groove. A real deck does start at the very rim, but the headshell is a solid
 * body extending back toward the pivot, so at the rim it overhangs the edge
 * and the head reads as sitting outside the vinyl. Measured: at a lead-in of
 * 294 the headshell's rear corner landed at 324.7, some 22 units beyond the
 * rim.
 *
 * The value tracks the headshell's size, so it is measured, not chosen: at 260
 * the rear corner sits at 299.7, just inside the rim. Enlarging the head means
 * re-measuring this alongside ARM_LEN.
 */
const R_LEAD_IN = 260;
const R_RUN_OUT = 128;

const BASE_ANGLE =
  (Math.atan2(CENTER.y - PIVOT.y, CENTER.x - PIVOT.x) * 180) / Math.PI;

function armRotationFor(radius) {
  const cos =
    (ARM_LEN * ARM_LEN + PIVOT_DIST * PIVOT_DIST - radius * radius) /
    (2 * ARM_LEN * PIVOT_DIST);
  const theta = (Math.acos(Math.min(1, Math.max(-1, cos))) * 180) / Math.PI;
  return BASE_ANGLE - theta - DRAWN_ANGLE;
}

/**
 * Parked, when there is no session at all. Derived, not guessed: a stylus
 * radius of 340 is beyond the record's 303, so the arm swings clear of the
 * vinyl rather than hovering over a record that is not there.
 */
const REST_ROTATION = armRotationFor(340);

/* ══════════════════════════════════════════════════════════════════════
 * State
 *
 * `positionMs` is the position as of `updatedAt`, not a live value. SMTC
 * sources leave it untouched for minutes; Phase 0 measured Edge holding one
 * for over four minutes. All smooth motion is extrapolated here.
 * See CLAUDE.md constraint 1.
 * ══════════════════════════════════════════════════════════════════════ */
let snapshot = null;
let shownMs = null;
let labelKey = null;

/** Corrections smaller than this are absorbed rather than snapped to. */
const CORRECTION_TOLERANCE_MS = 1500;

/* ══════════════════════════════════════════════════════════════════════
 * Rotation. One composited animation, rate-ramped by a rAF that terminates.
 * Nothing writes the platter transform per frame.
 * ══════════════════════════════════════════════════════════════════════ */
const spin = spinG.animate(
  [{ transform: "rotate(0deg)" }, { transform: "rotate(360deg)" }],
  { duration: 1800, iterations: Infinity, easing: "linear" },
);
spin.playbackRate = 0;
spin.pause();

let ramp = null;
let lastRate = null;

function rampSpinTo(target) {
  if (ramp !== null) cancelAnimationFrame(ramp);

  if (RM.matches) {
    spin.playbackRate = target;
    if (target === 0) spin.pause();
    else spin.play();
    ramp = null;
    return;
  }

  // Direct drive reaches speed quickly and coasts down slowly.
  const tau = target > 0 ? 550 : 950;
  let last = performance.now();
  if (target > 0) spin.play();

  const step = (now) => {
    const dt = Math.min(64, now - last);
    last = now;
    const rate =
      spin.playbackRate +
      (target - spin.playbackRate) * (1 - Math.exp(-dt / tau));

    if (Math.abs(target - rate) < 0.012) {
      spin.playbackRate = target;
      if (target === 0) spin.pause();
      ramp = null;
      return;
    }
    spin.playbackRate = rate;
    ramp = requestAnimationFrame(step);
  };
  ramp = requestAnimationFrame(step);
}

/* ══════════════════════════════════════════════════════════════════════
 * Grooves. Seeded, so the record presses identically every launch.
 * ══════════════════════════════════════════════════════════════════════ */
function seededRandom(seed) {
  let v = seed;
  return () => {
    v = (v * 1664525 + 1013904223) % 4294967296;
    return v / 4294967296;
  };
}

function buildGrooves() {
  const NS = "http://www.w3.org/2000/svg";
  const grooves = $("grooves");
  const frag = document.createDocumentFragment();

  const ring = (r, opacity, width, sep) => {
    const c = document.createElementNS(NS, "circle");
    c.setAttribute("cx", CENTER.x);
    c.setAttribute("cy", CENTER.y);
    c.setAttribute("r", r.toFixed(2));
    c.setAttribute("fill", "none");
    c.setAttribute("stroke-width", width);
    c.setAttribute("opacity", opacity.toFixed(3));
    if (sep) c.setAttribute("class", "sep");
    frag.appendChild(c);
  };

  // Varied pitch rather than uniform rings, which is what stops it reading as
  // graph paper. Deterministic, so the record presses identically every launch.
  for (let r = 126; r <= 296; r += 1.7) {
    const opacity = 0.14 + 0.1 * Math.abs(Math.sin(r * 0.33)) + (((r * 7919) % 13) / 13) * 0.05;
    ring(r, opacity, 1.05, false);
  }
  // The wider bands a real pressing shows between tracks.
  [152, 180, 208, 236, 262, 286].forEach((r) => ring(r, 1, 2.6, true));
  ring(298.5, 1, 3, true); // lead-in

  grooves.appendChild(frag);
}

/* ══════════════════════════════════════════════════════════════════════
 * Lyrics
 *
 * Rust fetches and parses; this scrolls. Lines arrive as {atMs, text} and are
 * laid out once per track, after which the only work per line is one transform
 * on the track element and one class swap.
 *
 * That matters: a lyric lands every few seconds, so this writes far less often
 * than the 4 Hz timecode beside it. CLAUDE.md constraint 4 rules out per-frame
 * DOM work, and an auto-scrolling lyric is exactly the feature that invites it.
 * ══════════════════════════════════════════════════════════════════════ */

/** Where in the viewport the sung line sits, as a fraction of its height.
 *  A third down rather than centred, so more of what is coming is visible. */
const LYRIC_ANCHOR = 0.34;

let lyrics = null;
/** Offset of each line within the track, measured once after layout. */
let lyricOffsets = [];
let lyricIndex = -1;

function hasLyrics() {
  return Boolean(lyrics && lyrics.lines && lyrics.lines.length);
}

/**
 * Lays the lines out for a new track.
 *
 * The one expensive step, and it runs on a track change rather than on a tick.
 * Offsets are read back immediately afterwards so the scroll never has to
 * measure again: reading offsetTop per line during playback would force a
 * layout four times a second.
 */
function buildLyrics(next) {
  lyrics = next;
  lyricIndex = -1;
  lyricOffsets = [];

  const track = $("uiLyricTrack");

  if (!hasLyrics()) {
    track.textContent = "";
    track.style.transform = "translateY(0)";
    widget.classList.remove("has-lyrics");
    return;
  }

  track.innerHTML = lyrics.lines
    .map((line) => `<p>${escapeHtml(line.text)}</p>`)
    .join("");
  widget.classList.add("has-lyrics");

  const rows = track.children;
  for (let i = 0; i < rows.length; i += 1) {
    lyricOffsets.push(rows[i].offsetTop + rows[i].offsetHeight / 2);
  }

  syncLyric(currentPositionMs() ?? 0);
}

function escapeHtml(value) {
  return String(value).replace(
    /[<>&]/g,
    (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" })[c],
  );
}

/** The last line whose stamp has passed, or -1 before the first one. */
function lyricAt(positionMs) {
  const lines = lyrics.lines;
  let low = 0;
  let high = lines.length - 1;
  let found = -1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (lines[mid].atMs <= positionMs) {
      found = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }
  return found;
}

/** Moves the scroll if, and only if, the sung line has changed. */
function syncLyric(positionMs) {
  if (!hasLyrics()) return;

  const next = lyricAt(positionMs);
  if (next === lyricIndex) return;

  const track = $("uiLyricTrack");
  const rows = track.children;
  if (lyricIndex >= 0 && rows[lyricIndex]) rows[lyricIndex].classList.remove("on");
  lyricIndex = next;
  if (next >= 0 && rows[next]) rows[next].classList.add("on");

  // Before the first stamp, sit at the top rather than scrolling the intro
  // off-screen.
  const centre = next >= 0 ? lyricOffsets[next] : 0;
  const anchor = $("uiLyrics").clientHeight * LYRIC_ANCHOR;
  const shift = Math.max(0, centre - anchor);
  track.style.transform = `translateY(${-shift.toFixed(1)}px)`;
}

/* ══════════════════════════════════════════════════════════════════════
 * The label
 * ══════════════════════════════════════════════════════════════════════ */
function artUrlFor(artId) {
  // Custom scheme, so the bytes never cross the bridge as base64. On Windows
  // this resolves to http://art.localhost/{id}.
  return convertFileSrc(artId, "art");
}

/**
 * Swaps the label, keyed so it only rebuilds when the track actually changes.
 * Rebuilding every tick would defeat the whole art_id caching scheme.
 */
function syncLabel() {
  const key = snapshot
    ? [
        snapshot.artId ?? "",
        snapshot.title ?? "",
        snapshot.artist ?? "",
        snapshot.sourceApp ?? "",
      ].join("|")
    : "none";

  if (key === labelKey) return;
  const first = labelKey === null;
  labelKey = key;

  const paint = () => {
    if (!snapshot || snapshot.status === "noSession") {
      labelG.innerHTML = "";
    } else if (snapshot.artId) {
      labelG.innerHTML = artLabel(artUrlFor(snapshot.artId));
    } else {
      labelG.innerHTML = proceduralLabel({
        title: snapshot.title,
        artist: snapshot.artist,
        sourceApp: snapshot.sourceApp,
      });
      fitLabelText(labelG);
    }
    labelG.style.opacity = "1";
  };

  // Fade down, swap, fade up: a track change must not flash empty.
  if (first || RM.matches) {
    paint();
    return;
  }
  labelG.style.opacity = "0";
  setTimeout(paint, 200);
}

/* ══════════════════════════════════════════════════════════════════════
 * Screen tint
 *
 * The screen takes its hue from the cover art, so After Hours gives a red
 * screen and so on. Only the hue is taken: saturation and lightness are pinned
 * to a narrow band, because the text on the screen is white and an unclamped
 * cover would eventually produce pale yellow and make it unreadable. The
 * result stays "dusty" like the reference and only ever shifts in hue.
 * ══════════════════════════════════════════════════════════════════════ */
/** Lightness and saturation bands that keep white type legible. */
function screenBand() {
  // The resolved palette, not the system one: with the theme forced to Light on
  // a dark desktop, a dark band would put dim type on a bright screen.
  return isDark()
    ? { light: 0.22, sat: [0.14, 0.32] }
    : { light: 0.6, sat: [0.16, 0.36] };
}

function hslCss(h, s, l) {
  return `hsl(${h.toFixed(1)} ${(s * 100).toFixed(1)}% ${(l * 100).toFixed(1)}%)`;
}

function applyTint(hue, saturation) {
  const band = screenBand();
  const s = Math.min(band.sat[1], Math.max(band.sat[0], saturation));
  const root = document.documentElement.style;
  root.setProperty("--screen-hi", hslCss(hue, s, band.light + 0.045));
  root.setProperty("--screen", hslCss(hue, s, band.light));
  root.setProperty("--screen-lo", hslCss(hue, s, band.light - 0.045));
}

/**
 * Finds the dominant hue of an image.
 *
 * Sampled at 24x24, which is ample for a hue and costs nothing. Near-black,
 * near-white and washed-out pixels are skipped: a cover that is mostly a dark
 * background should be judged on its actual colour, not on the background.
 */
function dominantHue(image) {
  const size = 24;
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(image, 0, 0, size, size);

  const { data } = ctx.getImageData(0, 0, size, size);

  // 24 hue buckets, weighted by saturation so a small vivid area outvotes a
  // large muddy one, which is how a person would read a cover.
  const buckets = new Float64Array(24);
  const sats = new Float64Array(24);
  const counts = new Float64Array(24);
  let counted = 0;

  for (let i = 0; i < data.length; i += 4) {
    const r = data[i] / 255;
    const g = data[i + 1] / 255;
    const b = data[i + 2] / 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const l = (max + min) / 2;
    if (l < 0.08 || l > 0.94) continue; // near-black or blown out
    const d = max - min;
    if (d < 0.08) continue; // effectively grey
    const s = d / (1 - Math.abs(2 * l - 1));

    let h;
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h = (h * 60 + 360) % 360;

    const bucket = Math.floor(h / 15);
    buckets[bucket] += s * s;
    sats[bucket] += s;
    counts[bucket] += 1;
    counted += 1;
  }

  if (!counted) return null;

  let best = 0;
  for (let i = 1; i < buckets.length; i += 1) {
    if (buckets[i] > buckets[best]) best = i;
  }
  if (buckets[best] <= 0) return null;

  return {
    hue: best * 15 + 7.5,
    saturation: sats[best] / counts[best],
  };
}

let tintKey = null;

function syncTint() {
  const key = snapshot
    ? `${snapshot.artId ?? ""}|${snapshot.title ?? ""}|${snapshot.artist ?? ""}`
    : "none";
  if (key === tintKey) return;
  tintKey = key;

  if (!snapshot || snapshot.status === "noSession") {
    // Back to the default dusty rose rather than holding the last track's hue.
    for (const name of ["--screen-hi", "--screen", "--screen-lo"]) {
      document.documentElement.style.removeProperty(name);
    }
    return;
  }

  if (!snapshot.artId) {
    // No cover: take the hue from the procedural label instead, so the screen
    // and the record still belong to each other.
    applyTint(
      proceduralHue({
        title: snapshot.title,
        artist: snapshot.artist,
        sourceApp: snapshot.sourceApp,
      }),
      0.26,
    );
    return;
  }

  const image = new Image();
  image.crossOrigin = "anonymous";
  image.onload = () => {
    try {
      const found = dominantHue(image);
      if (found) applyTint(found.hue, found.saturation);
    } catch (error) {
      // A tainted canvas or a decode failure is not worth breaking over; the
      // screen simply keeps its default colour.
      console.warn("could not sample cover art for the screen tint", error);
    }
  };
  image.src = artUrlFor(snapshot.artId);
}

/* ══════════════════════════════════════════════════════════════════════
 * Transport
 *
 * The click updates the UI immediately and the real SMTC event reconciles it
 * a moment later. Phase 0 measured Edge taking minutes to volunteer a timeline
 * update, so waiting for confirmation would make the buttons feel broken.
 * ══════════════════════════════════════════════════════════════════════ */
async function send(action) {
  try {
    await invoke("transport", { action });
  } catch (error) {
    console.error(`transport ${action} failed`, error);
    render(); // the optimistic guess was wrong; fall back to real state
  }
}

function toggle() {
  if (!snapshot || !snapshot.canPlayPause) return;

  // Optimistic flip. Re-anchor to now so the extrapolation does not jump when
  // the real update arrives.
  const position = currentPositionMs();
  snapshot = {
    ...snapshot,
    status: snapshot.status === "playing" ? "paused" : "playing",
    positionMs: position ?? snapshot.positionMs,
    updatedAt: Date.now(),
  };
  render();
  send("toggle");
}

function skip(action) {
  const allowed = action === "next" ? snapshot?.canNext : snapshot?.canPrevious;
  if (!allowed) return;
  send(action);
}

function bindTransport() {
  $("btnPlay").addEventListener("click", toggle);
  $("btnNext").addEventListener("click", () => skip("next"));
  $("btnPrev").addEventListener("click", () => skip("previous"));
  // Inert unless the widget is compact; CSS keeps it out of the way otherwise.
  $("deckHit").addEventListener("click", toggle);
}

/* ══════════════════════════════════════════════════════════════════════
 * Render
 * ══════════════════════════════════════════════════════════════════════ */
function formatTime(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  if (m >= 60) {
    return `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${m}:${String(s).padStart(2, "0")}`;
}

/**
 * Extrapolates the true position from the anchor, then guards the transition.
 *
 * A source can re-anchor slightly behind where we had already counted to.
 * Holding rather than snapping is what stops the tonearm twitching backwards;
 * a genuinely large jump is a real seek and is obeyed immediately.
 */
function currentPositionMs() {
  if (!snapshot || snapshot.positionMs === null) return null;

  let target = snapshot.positionMs;
  if (snapshot.status === "playing") {
    target += Date.now() - snapshot.updatedAt;
  }
  if (snapshot.durationMs) target = Math.min(target, snapshot.durationMs);
  target = Math.max(0, target);

  if (shownMs === null) {
    shownMs = target;
  } else {
    const delta = target - shownMs;
    if (delta >= 0 || -delta >= CORRECTION_TOLERANCE_MS) shownMs = target;
  }
  return shownMs;
}

let lastRotation = null;

function render() {
  const hasSession = Boolean(snapshot) && snapshot.status !== "noSession";
  const playing = hasSession && snapshot.status === "playing";
  // A livestream reports no duration. Park the arm rather than inventing a
  // position.
  const isLive = hasSession && !snapshot.durationMs;

  widget.classList.toggle("loaded", hasSession);
  widget.classList.toggle("playing", playing);
  widget.classList.toggle("live", isLive);

  const wantRate = playing ? 1 : 0;
  if (wantRate !== lastRate) {
    lastRate = wantRate;
    rampSpinTo(wantRate);
  }

  const position = currentPositionMs();
  const fraction =
    !isLive && snapshot?.durationMs && position !== null
      ? Math.min(1, position / snapshot.durationMs)
      : 0;

  const radius = isLive
    ? R_LEAD_IN
    : R_LEAD_IN + (R_RUN_OUT - R_LEAD_IN) * fraction;

  // Only write when it actually moves; this runs at 4 Hz.
  const rotation = hasSession ? armRotationFor(radius) : REST_ROTATION;
  if (rotation !== lastRotation) {
    lastRotation = rotation;
    armG.setAttribute(
      "transform",
      `rotate(${rotation.toFixed(3)} ${PIVOT.x} ${PIVOT.y})`,
    );
  }

  // Only offer what the source says it supports.
  const canToggle = hasSession && snapshot.canPlayPause;
  $("btnPlay").disabled = !canToggle;
  $("btnNext").disabled = !hasSession || !snapshot.canNext;
  $("btnPrev").disabled = !hasSession || !snapshot.canPrevious;
  $("btnPlay").setAttribute("aria-label", playing ? "Pause" : "Play");
  $("deckHit").disabled = !canToggle;
  $("deckHit").setAttribute("aria-label", playing ? "Pause" : "Play");

  if (!hasSession) {
    $("uiTitle").textContent = "No session";
    $("uiArtist").textContent = "Nothing is playing";
    $("uiAlbum").textContent = "";
    $("uiElapsed").textContent = "0:00";
    $("uiTotal").textContent = "–:––";
    setProgress(0);
    return;
  }

  $("uiTitle").textContent = snapshot.title || "Unknown";
  $("uiArtist").textContent = snapshot.artist || "Unknown artist";
  // Only worth showing when it adds something the artist line does not.
  $("uiAlbum").textContent =
    snapshot.album && snapshot.album !== snapshot.title ? snapshot.album : "";

  if (isLive) {
    // Showing "0:00 / 0:00" would be a lie.
    $("uiElapsed").textContent = "LIVE";
    $("uiTotal").textContent = "";
    setProgress(0);
  } else {
    $("uiElapsed").textContent = formatTime(position ?? 0);
    $("uiTotal").textContent = formatTime(snapshot.durationMs ?? 0);
    setProgress(fraction);
  }

  // Cheap: a binary search over the stamps, and it writes only when the sung
  // line actually changes, which is every few seconds rather than every tick.
  syncLyric(position ?? 0);
}

function setProgress(fraction) {
  $("uiBar").style.width = `${(fraction * 100).toFixed(2)}%`;
}

/* ══════════════════════════════════════════════════════════════════════
 * Preferences
 *
 * Size and palette both come from the right-click menu, which Rust owns: it
 * resizes the window, writes config.json and emits `prefs-changed`. All this
 * side does is restyle.
 * ══════════════════════════════════════════════════════════════════════ */
const SYSTEM_DARK = matchMedia("(prefers-color-scheme: dark)");
const root = document.documentElement;

/** The palette actually in force, which is what every caller wants. */
function isDark() {
  return root.dataset.theme === "dark";
}

/**
 * Writes the resolved palette to the root.
 *
 * "auto" is never written: styles.css declares its dark tokens once, under
 * `[data-theme="dark"]`, so the choice has to be resolved to a concrete palette
 * here rather than left for a media query to settle.
 */
function applyTheme(theme) {
  const dark = theme === "dark" || (theme === "auto" && SYSTEM_DARK.matches);
  root.dataset.theme = dark ? "dark" : "light";
  // The tint's lightness band is keyed off the palette, so a theme change has
  // to recompute it or the screen keeps the other palette's band.
  syncTint();
}

function applyPrefs(prefs) {
  theme = prefs.theme;
  applyTheme(theme);
  widget.classList.toggle("compact", prefs.size === "compact");
}

let theme = "auto";

// Only meaningful while the choice is Match Windows; harmless otherwise, since
// applyTheme re-resolves from the current choice either way.
SYSTEM_DARK.addEventListener("change", () => {
  if (theme === "auto") applyTheme(theme);
});

// The webview swallows right-click by default, so the menu has to be asked for.
addEventListener("contextmenu", (event) => {
  event.preventDefault();
  invoke("show_menu").catch((error) => console.error("show_menu failed", error));
});

function adopt(next) {
  const changedTrack =
    !snapshot ||
    snapshot.title !== next.title ||
    snapshot.sourceApp !== next.sourceApp ||
    next.status === "noSession";

  snapshot = next;
  if (changedTrack) {
    shownMs = null;
    // Rust sends the new track's lyrics a moment later. Clearing here rather
    // than waiting stops the old song's words sitting under the new title for
    // however long the lookup takes.
    buildLyrics(null);
  }

  syncLabel();
  syncTint();
  render();
}

/* ══════════════════════════════════════════════════════════════════════
 * Boot
 * ══════════════════════════════════════════════════════════════════════ */
buildGrooves();
bindTransport();

await listen("playback-changed", (event) => adopt(event.payload));
await listen("prefs-changed", (event) => applyPrefs(event.payload));

// Applied as sent. The worker is a single thread that emits a clear before each
// lookup and the result after it, so the order is always clear(A), result(A),
// clear(B), result(B) and a reply can never overtake the track it belongs to.
//
// An earlier version re-derived Rust's track key here and dropped anything that
// did not match. That duplicated a format defined in `lyrics.rs`, which is
// exactly the kind of agreement that silently rots: the two only had to differ
// by a space for every lookup to be discarded and no lyric to ever appear.
await listen("lyrics-changed", (event) => buildLyrics(event.payload));

// Before the first render: the saved size decides the layout, and restyling
// after paint would show the wrong one for a frame.
try {
  applyPrefs(await invoke("get_prefs"));
} catch (error) {
  console.error("get_prefs failed; keeping the defaults", error);
}

try {
  adopt(await invoke("get_state"));
} catch (error) {
  console.error("initial get_state failed", error);
}

// After adopt, which clears them: Rust reaches a track long before the webview
// is ready, so the first lyrics-changed usually fires with nobody listening.
// Without this read the song playing at launch never gets its words.
try {
  buildLyrics(await invoke("get_lyrics"));
} catch (error) {
  console.error("initial get_lyrics failed", error);
}

// 4 Hz. The tonearm updates at most twice a second per CLAUDE.md constraint 4;
// this is double that so the timecode ticks cleanly. Nothing here touches the
// platter, which runs on the compositor.
setInterval(render, 250);
