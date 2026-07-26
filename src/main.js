import { artLabel, fitLabelText, proceduralLabel } from "./label.js";

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const RM = matchMedia("(prefers-reduced-motion: reduce)");

const widget = $("widget");
const spinG = $("spinG");
const armG = $("armG");
const labelG = $("labelG");
const needleSh = $("needleSh");

/* ══════════════════════════════════════════════════════════════════════
 * Geometry
 *
 * Platter centre (96,112), record r 72, label r 26, arm pivot (198,42).
 * The arm is drawn pointing straight down, so a rotation of 0 means "along
 * +y". The engagement angle is derived by intersecting the stylus arc with
 * the groove radius, so the stylus genuinely tracks the playhead.
 * ══════════════════════════════════════════════════════════════════════ */
const CENTER = { x: 96, y: 112 };
const PIVOT = { x: 198, y: 42 };
const RECORD_R = 72;
const LABEL_R = 26;
/**
 * Pivot to stylus, and the bearing of the stylus in the arm's drawn pose.
 *
 * The contact point is the underside of the cartridge at (198, 137.4), not a
 * drawn stylus: seen from directly above, the stylus is hidden beneath the
 * cartridge, so nothing is drawn there. The headshell is then rotated 21
 * degrees at the offset angle, which swings that point off the tube's axis to
 * (193.20, 136.51).
 *
 * So the arm is not simply "pointing down": pivot to contact measures 94.63 at
 * 92.91 degrees, not 98 at 90. Both are derived from the drawn geometry,
 * because assuming 90 would leave the cartridge sitting several degrees off
 * the groove it is meant to be tracking.
 */
const ARM_LEN = 94.63;
const PIVOT_DIST = Math.hypot(PIVOT.x - CENTER.x, PIVOT.y - CENTER.y);

const R_LEAD_IN = RECORD_R * 0.92;
const R_RUN_OUT = LABEL_R + 3;

const BASE_ANGLE =
  (Math.atan2(CENTER.y - PIVOT.y, CENTER.x - PIVOT.x) * 180) / Math.PI;
const DRAWN_ANGLE = 92.91;

/**
 * Parked. Zero, not an offset: the arm is drawn pointing straight down from
 * the pivot, and the arm rest sits directly beneath it at (198, 120), so the
 * tube lies across the rest exactly when the rotation is zero. Any offset
 * swings the stylus off the rest and out over bare plinth.
 */
const REST_ROTATION = 0;

function armRotationFor(radius) {
  const cos =
    (ARM_LEN * ARM_LEN + PIVOT_DIST * PIVOT_DIST - radius * radius) /
    (2 * ARM_LEN * PIVOT_DIST);
  const theta = (Math.acos(Math.min(1, Math.max(-1, cos))) * 180) / Math.PI;
  return BASE_ANGLE - theta - DRAWN_ANGLE;
}

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
  const rand = seededRandom(20220517);

  const ring = (r, opacity, width) => {
    const c = document.createElementNS(NS, "circle");
    c.setAttribute("cx", CENTER.x);
    c.setAttribute("cy", CENTER.y);
    c.setAttribute("r", r.toFixed(2));
    c.setAttribute("stroke", "#8f96a2");
    c.setAttribute("stroke-width", width);
    c.setAttribute("opacity", opacity.toFixed(3));
    grooves.appendChild(c);
  };

  for (let r = R_RUN_OUT; r <= R_LEAD_IN; r += 1.05 + rand() * 0.5) {
    ring(r, 0.055 + rand() * 0.05, 0.6);
  }
  [40.5, 50.5, 59.5].forEach((r) => ring(r, 0.21, 1)); // track gaps
  ring(R_LEAD_IN + 1.4, 0.17, 1); // lead-in
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
  // A livestream reports no duration. Phase 0 never caught one, so this is
  // implemented from the spec: park the arm, do not invent a position.
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
    armG.setAttribute("transform", `rotate(${rotation.toFixed(3)} 198 42)`);
  }
  needleSh.setAttribute("opacity", playing ? "0.5" : "0");

  // Only offer what the source says it supports.
  $("btnPlay").disabled = !hasSession || !snapshot.canPlayPause;
  $("btnNext").disabled = !hasSession || !snapshot.canNext;
  $("btnPrev").disabled = !hasSession || !snapshot.canPrevious;
  $("btnPlay").setAttribute("aria-label", playing ? "Pause" : "Play");
  $("cue").style.cursor =
    hasSession && snapshot.canPlayPause ? "pointer" : "default";

  if (!hasSession) {
    $("uiTitle").textContent = "No session";
    $("uiArtist").textContent = "Nothing is playing";
    $("uiSource").textContent = "";
    $("uiElapsed").textContent = "0:00";
    $("uiTotal").textContent = "–:––";
    $("uiBar").style.width = "0%";
    return;
  }

  $("uiTitle").textContent = snapshot.title || "Unknown";
  $("uiArtist").textContent =
    snapshot.artist || snapshot.album || snapshot.sourceApp || "Unknown artist";
  $("uiSource").textContent = snapshot.sourceApp || "";

  if (isLive) {
    // Showing "0:00 / 0:00" would be a lie.
    $("uiElapsed").textContent = "LIVE";
    $("uiTotal").textContent = "";
    $("uiBar").style.width = "0%";
  } else {
    $("uiElapsed").textContent = formatTime(position ?? 0);
    $("uiTotal").textContent = formatTime(snapshot.durationMs ?? 0);
    $("uiBar").style.width = `${(fraction * 100).toFixed(2)}%`;
  }
}

/* ══════════════════════════════════════════════════════════════════════
 * Transport
 *
 * The click updates the UI immediately and the real SMTC event reconciles it
 * a moment later. Phase 0 measured Edge taking minutes to volunteer a timeline
 * update, so waiting for confirmation would make the button feel broken.
 * ══════════════════════════════════════════════════════════════════════ */
async function send(action) {
  try {
    await invoke("transport", { action });
  } catch (error) {
    console.error(`transport ${action} failed`, error);
    // The optimistic guess was wrong; fall back to whatever Rust last told us.
    render();
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
  // The cue lever, which is what actually raises the arm on a deck. The record
  // itself used to toggle too, but three ways to pause was one too many and
  // clicking the platter was the least discoverable of them.
  $("cue").addEventListener("click", toggle);
}

function adopt(next) {
  const changedTrack =
    !snapshot ||
    snapshot.title !== next.title ||
    snapshot.sourceApp !== next.sourceApp ||
    next.status === "noSession";

  snapshot = next;
  if (changedTrack) shownMs = null;

  syncLabel();
  render();
}

/* ══════════════════════════════════════════════════════════════════════
 * Boot
 * ══════════════════════════════════════════════════════════════════════ */
buildGrooves();
bindTransport();

await listen("playback-changed", (event) => adopt(event.payload));

try {
  adopt(await invoke("get_state"));
} catch (error) {
  console.error("initial get_state failed", error);
}

// 4 Hz. The tonearm updates at most twice a second per CLAUDE.md constraint 4;
// this is double that so the timecode ticks cleanly. Nothing here touches the
// platter, which runs on the compositor.
setInterval(render, 250);
