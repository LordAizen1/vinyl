const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/**
 * The last snapshot from Rust. `positionMs` is the position as of `updatedAt`,
 * not a live value: SMTC sources leave it untouched for minutes at a time. All
 * smooth motion below is extrapolated locally. See CLAUDE.md constraint 1.
 */
let snapshot = null;

/** What we last displayed, so a correction never runs the clock backwards. */
let shownMs = null;

/** Corrections smaller than this are absorbed rather than snapped to. */
const CORRECTION_TOLERANCE_MS = 1500;

const fields = {};

function formatTime(totalMs) {
  if (totalMs === null || totalMs === undefined) return "—";

  const totalSeconds = Math.max(0, Math.floor(totalMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;

  if (minutes >= 60) {
    const hours = Math.floor(minutes / 60);
    const rest = minutes % 60;
    return `${hours}:${String(rest).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }

  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

/**
 * Extrapolates the true position from the anchor, then guards the transition.
 *
 * A source can re-anchor slightly behind where we had already counted to. Easing
 * over that rather than snapping is what stops the tonearm twitching backwards;
 * a genuinely large jump is a real seek and should be obeyed immediately.
 */
function currentPositionMs() {
  if (!snapshot || snapshot.positionMs === null) return null;

  let target = snapshot.positionMs;
  if (snapshot.status === "playing") {
    target += Date.now() - snapshot.updatedAt;
  }

  if (snapshot.durationMs) {
    target = Math.min(target, snapshot.durationMs);
  }
  target = Math.max(0, target);

  if (shownMs === null) {
    shownMs = target;
    return shownMs;
  }

  const delta = target - shownMs;
  if (delta >= 0) {
    shownMs = target;
  } else if (-delta >= CORRECTION_TOLERANCE_MS) {
    // Large backward move: a real seek. Obey it.
    shownMs = target;
  }
  // Otherwise hold, and let real time catch the anchor up.

  return shownMs;
}

function render() {
  if (!snapshot || snapshot.status === "noSession") {
    fields.status.textContent = "no session";
    for (const key of ["title", "artist", "album", "position", "source", "anchor"]) {
      fields[key].textContent = "—";
    }
    return;
  }

  fields.status.textContent = snapshot.status;
  fields.title.textContent = snapshot.title ?? "—";
  fields.artist.textContent = snapshot.artist ?? "—";
  fields.album.textContent = snapshot.album ?? "—";
  fields.source.textContent = snapshot.sourceApp || "—";

  const position = currentPositionMs();
  if (snapshot.durationMs) {
    fields.position.textContent = `${formatTime(position)} / ${formatTime(snapshot.durationMs)}`;
  } else {
    // Livestreams report no duration. Showing "0:00 / 0:00" would be a lie.
    fields.position.textContent = `${formatTime(position)} / live`;
  }

  const ageSeconds = (Date.now() - snapshot.updatedAt) / 1000;
  fields.anchor.textContent = `${ageSeconds.toFixed(1)}s`;
}

function adopt(next) {
  // A different track, or a seek, resets the guard so the new anchor is trusted.
  const changedTrack =
    !snapshot ||
    snapshot.title !== next.title ||
    snapshot.sourceApp !== next.sourceApp ||
    next.status === "noSession";

  snapshot = next;
  if (changedTrack) shownMs = null;

  render();
}

window.addEventListener("DOMContentLoaded", async () => {
  for (const key of ["status", "title", "artist", "album", "position", "source", "anchor"]) {
    fields[key] = document.querySelector(`#${key}`);
  }

  await listen("playback-changed", (event) => adopt(event.payload));

  try {
    adopt(await invoke("get_state"));
  } catch (error) {
    console.error("initial get_state failed", error);
  }

  // 250 ms is plenty for text. The real tonearm updates twice a second.
  setInterval(render, 250);
});
