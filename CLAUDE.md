# CLAUDE.md

Persistent context for this repo. Read `PLAN.md` for the phased roadmap and acceptance
criteria, and `ASSETS.md` before adding any file you did not write — fonts, textures,
icons, or code snippets. All geometry in this project is hand-authored SVG; sourcing a
turntable image or an existing vinyl-player component is explicitly out of bounds.

## What we're building

A desktop widget for Windows: a small vinyl record player that sits in a corner of the
screen and shows whatever audio is currently playing anywhere on the system — Spotify,
a YouTube tab, VLC, a podcast app, a game. The platter spins at a true 33⅓ RPM, the
album art is the record's label, and the tonearm creeps inward as the track progresses.
Clicking it can pause, skip, and resume the real source app.

Name: `vinyl` (working title).

## Non-goals

Do not build these. If you think one is needed, stop and ask first.

- No LLM or AI API calls anywhere in this project. It is fully offline and deterministic.
- No network requests of any kind, except optionally fetching fonts at build time.
- No music library, no playlists, no file scanning. We only reflect existing playback.
- No macOS or Linux support. Windows 10 1809+ only.
- No Electron. No bundled Chromium.
- No telemetry, no analytics, no crash reporting to a server.

## Stack

| Layer | Choice |
|---|---|
| Shell | Tauri v2 |
| Backend | Rust 2021 |
| Windows APIs | `windows` crate (windows-rs) |
| Frontend | Vanilla HTML + CSS + SVG. No React, no Vue, no build step beyond Vite's default |
| State | A single `PlaybackState` struct behind a `parking_lot::RwLock` |

Deliberately no frontend framework: the UI is one screen with a handful of animated
SVG nodes. A framework would add weight and a virtual DOM diff we do not need.

Expected crates: `tauri`, `windows`, `serde`, `serde_json`, `parking_lot`, `anyhow`,
`log`, `env_logger`. Later phases may add `wasapi` and `realfft`. Do not add
dependencies beyond these without asking.

## Architecture

Three strictly separated layers. Keep the boundaries clean — it is what makes this
testable and what lets the visual work happen without touching Rust.

```
┌─ smtc.rs ──────────┐
│ SMTC event thread  │──┐
│ (metadata, art,    │  │
│  position, status) │  │   ┌─ state.rs ─────┐      ┌─ frontend ────────┐
└────────────────────┘  ├──▶│  PlaybackState │─────▶│ SVG turntable     │
┌─ meter.rs ─────────┐  │   │  (RwLock)      │ emit │ CSS animation     │
│ Audio peak poller  │──┘   └────────────────┘      └───────────────────┘
│ (30 Hz)            │
└────────────────────┘
```

### The one state type

```rust
pub enum Status { Playing, Paused, Stopped, NoSession }

pub struct PlaybackState {
    pub title:       Option<String>,
    pub artist:      Option<String>,
    pub album:       Option<String>,
    pub art_id:      Option<String>,   // hash of art bytes; frontend fetches by id
    pub status:      Status,
    pub position_ms: Option<u64>,      // as of updated_at, NOT live
    pub duration_ms: Option<u64>,      // None or 0 means unknown (livestreams)
    pub updated_at:  u64,              // epoch ms when position was captured
    pub source_app:  String,           // AUMID, e.g. "Spotify.exe", "msedge.exe"
    pub peak:        f32,              // 0.0..1.0 from the audio meter
}
```

Album art bytes never cross the Tauri bridge as base64 on a hot path. Cache them in
Rust keyed by `art_id` and serve via a registered URI scheme protocol so the frontend
uses a plain `<image href="art://{id}">`.

Phase 0 measured a single Apple Music thumbnail at **1,022,489 bytes**, and sizes vary
by nearly 70x between tracks from the same app. So the *read* itself must be gated on
the track identity changing. Do not read thumbnail bytes on a timer in order to hash
them and detect a change; that is a megabyte per tick and it will not fit the idle CPU
budget in constraint 4.

## Hard-won constraints — read before writing code

These are the five things that will silently break this project. They are already
diagnosed; implement the stated fix.

### 1. SMTC `Position` does not tick

`TimelineProperties.Position` is a snapshot valid as of `LastUpdatedTime`, pushed by
the source app whenever it feels like it. Reading it on a timer gives a frozen value.

Always extrapolate:

```
effective = position_ms + (now_ms - updated_at)    // only when Status::Playing
effective = min(effective, duration_ms)
```

The frontend runs its own smooth timer from the last sync and re-anchors whenever a
fresh SMTC update arrives. Never let the tonearm jump backwards on a re-anchor —
interpolate if the correction is under 1500 ms.

Measured in Phase 0 and confirmed: YouTube in Edge held `Position` at `2:38` while its
anchor aged past 26 seconds, and the extrapolation reconstructed correct progress the
whole time. Anchors go stale for tens of seconds routinely. This is the normal case,
not an edge case.

**`updated_at` must come from `TimelineProperties.LastUpdatedTime`, converted from its
1601 epoch, not from the time we polled.** Anchoring to poll time restarts the
extrapolation from zero on every read and parks the arm permanently.

**When a source republishes a position identical to the one already held, do not
re-anchor.** Keep the existing local clock running. Re-anchor only on a track change, a
genuinely different position, or a seek. Without this, a source that refreshes its
timestamp without moving its position pins the arm at `0:00` for the whole track.
Implemented as `Anchor::observe` in `src-tauri/src/smtc.rs`, with tests covering a
republished position, a backward seek, and a track change at the same position.

### 2. Tauri has no per-region hit testing

`setIgnoreCursorEvents` is all-or-nothing for the whole window. There is no way to be
click-through on the transparent corners and clickable on the record.

~~**Phase 4 decision: ship with click-through OFF.**~~ **Reversed.** The widget now
ships **locked**: click-through ON for the whole window, with a "Lock in place" tick in
the tray menu to turn it off. A desktop widget that swallows clicks in the middle of
your wallpaper is a nuisance, and being inert is most of what makes it feel like part
of the desktop rather than an app parked on it.

All-or-nothing is exactly right for this, which is why the limitation above stopped
mattering: locked means every part of it is inert, so there is no region to
distinguish. The 60 Hz cursor-polling workaround is still not built and still should
not be.

Two consequences, both intended. While locked the transport buttons do not work, so
the widget is something you look at and the tray keeps the controls. And a locked
widget cannot be right-clicked, so the tray is the *only* way back in: it is not an
optional convenience.

### 3. Album art is often missing

Browser sessions, local files, and livestreams frequently have no thumbnail. A blank
label looks broken, not minimal.

**Fix: generate a procedural record label** from a seeded hash of `artist + title`,
so the same track always produces the same label. Draw from a small library of
mid-century label archetypes (concentric rings, arced type, a fake catalogue number,
a palette per archetype). This must look deliberate, not degraded. See `PLAN.md`
Phase 3 for detail. Treat this as a feature, not a fallback.

### 4. Animation must cost nothing

The widget runs 24/7. Idle CPU target: **under 1%**.

- Platter rotation is a pure CSS animation: `animation: spin 1.8s linear infinite`
  with `will-change: transform`. Pause it with `animation-play-state: paused`.
- Never rotate the platter from JavaScript per frame. Never `requestAnimationFrame`
  the record.
- The tonearm updates at most twice per second. That is plenty.
- The audio meter event carries one f32. Nothing else at 30 Hz.
- When `Status::NoSession`, stop all animation and drop the meter thread to 2 Hz.

The SMTC worker is event-driven, with one deliberate exception: a 5 s watchdog re-read
so a missed or unsubscribed event cannot strand the UI on stale state. Accepted in
Phase 1 because it reads metadata only. **Revisit it in Phase 3**, where thumbnails
enter the read path and a per-tick megabyte would not fit this budget.

WebView2 will cost 60–120 MB RSS. That is accepted and not a bug.

### 5. `GetCurrentSession()` is not authoritative, and browsers collapse tabs

Two separate findings from Phase 0, both about picking what to display.

Windows' current session is a poor choice. It kept selecting Edge while Windows Photos
was also `Playing`, and a paused Edge session stayed registered indefinitely after the
tab was left alone.

Worse, with the Apple Music desktop app merely **installed and idle**, Windows selected
it over a YouTube tab that was actively playing. That session had status `Opened`, an
empty title and artist, zero duration, no artwork, and a `LastUpdatedTime` that was
never set. Rendering it would have shown a blank record while music was audible. The
selection then flipped back to Edge mid-run with no user action, so it is not stable
either.

**Fix: select the session ourselves.** Enumerate `GetSessions()` and rank: `Playing`
beats `Paused` beats everything else; break ties by most recently updated timeline.
`Opened` means registered but not playing and must rank near the bottom. Fall back to
`GetCurrentSession()` only when the ranking is empty.

Separately, Edge (and therefore every Chromium browser) publishes **one** session for
the entire browser, repointing its metadata at whichever tab it considers active. Two
tabs playing at once produce one session, not two. Enumerating harder does not help;
there is no per-tab surface in SMTC.

**This is a documented limitation, not a bug to fix.** One record on one turntable is
the right model. Do not build tab-tracking, and do not add a browser extension.

Note also that VLC published no SMTC session at all in testing, so some audible media
is simply invisible to this pipeline. Phase 6 peak metering can show that *something*
is playing; it can never say what.

## Design direction

The visual is the product. Everything in Rust is plumbing. Budget time accordingly.

The subject is **hi-fi equipment**, not vintage paper. A turntable is a precision
machine: brushed aluminium, a black rubber mat, tinted acrylic, a warm pilot lamp.
Dark equipment also sits far better on an arbitrary desktop wallpaper than a light
panel does.

**Explicitly avoid** the default "vintage" palette of cream `#F4F1EA` with a serif
display face and a terracotta accent. It is where every generated design lands for
this brief and it will read as templated. Also avoid drop-shadowed glassmorphism.

**Two palettes since Phase 5, following the Windows light/dark setting.** The dark one
below is still the default and still the reasoning above. A silver, brushed-aluminium
scheme was added on request for light desktops, where a near-black slab reads as a hole
rather than as an object. Both live in `src/styles.css` as custom properties, switched
by a single `prefers-color-scheme` block.

Note the implementation trap: **Chromium does not resolve `var()` inside SVG
presentation attributes**, so `fill="var(--x)"` fails silently. Every colour in the deck
SVG is therefore applied by CSS class, not by a fill attribute. Breaking that rule
breaks theming without any error.

Starting tokens for the dark scheme (revise with justification, don't drift):

```
--plinth    #16171A   near-black, slight blue cast, the chassis
--vinyl     #0B0B0D   the record surface, darker than the plinth
--groove    #2A2C31   groove highlight rings, very low contrast
--steel     #8A8F98   brushed aluminium tonearm and hardware
--lamp      #FFB454   warm amber pilot lamp, the only warm colour
--oxblood   #6E1F26   accent, used once
```

Type roles:
- **UI text** — a condensed grotesque set in caps: title, artist, source.
  Sanitise the artist before setting it: Apple Music reports `Artist` as
  `The Weeknd — Dawn FM`, packing the album in, and YouTube reports channel names like
  `JJ Lin - Topic`. Both need trimming or the label carries the wrong words and
  overflows the text path. `AlbumTitle` and `AlbumArtist` are no help; Apple Music
  leaves the former empty and repeats the packed string in the latter. Split once on
  ` — ` and strip Apple's ` - Single` and ` - EP` suffixes.
- **Timecode** — a mono face with tabular figures so digits don't jitter. Small.
- **Display** — a medieval blackletter, added in Phase 5 on request. Two uses only: the
  `Vinyl` brand plate on the plinth, where a real deck carries the maker's name, and the
  large initial on the procedural label, where a gothic capital is an old convention.
  **Never in all caps**: at plinth size, blackletter capitals collapse into an
  unreadable smear, so the wordmark is mixed case. The label's small arced line stays in
  the grotesque, because it renders at around 3.5px. This is the one place the "avoid a
  serif display face" line above is deliberately overridden; it was aimed at the whole
  cream-and-terracotta package, which this is not. Do not let it spread further.
- No third face.

**Signature element: the needle drop.** On play, the tonearm swings over and settles,
with a half-second of surface noise before the music reads as "started". On pause, it
lifts. This is the one memorable thing; keep everything around it quiet. Respect
`prefers-reduced-motion` by cutting the swing to an instant cut.

Details that separate a good turntable from a bad one, in priority order:
1. True 1.8 s per rotation. Anything faster reads as a cartoon.
2. The tonearm pivots, so the stylus traces a slight **arc**, not a radial line.
3. A soft shadow cast by the arm onto the record surface, moving with it.
4. A specular highlight that sweeps the vinyl as it turns.
5. Grooves with varied spacing, not uniform rings.
6. The label is legible for one instant per rotation. That is the charm, not a bug.

## Conventions

- `cargo clippy -- -D warnings` must pass. `cargo fmt` before every commit.
- All Windows API calls are wrapped in a safe Rust function in the module that owns
  them. `unsafe` blocks stay as small as possible and never leak into `state.rs` or
  the Tauri command layer.
- Every Windows call that can fail returns `anyhow::Result`. Never `.unwrap()` on a
  COM or WinRT call — a user unplugging headphones must not crash the widget.
- WinRT/COM must be initialised per worker thread. Do it once at thread start.
- Log to a file in `%LOCALAPPDATA%\vinyl\vinyl.log`, capped. No stdout in release.
- Config (window position, RPM preference, always-on-top) in
  `%APPDATA%\vinyl\config.json`. Write on change, debounced.
- Commit messages: imperative mood, one line, scoped, e.g. `smtc: extrapolate position`.

## Working agreement

- **Stop at every phase gate** in `PLAN.md` and wait for review before starting the
  next phase. Do not run phases together.
- At each gate, state what you built, how you verified it, and anything you found
  that contradicts this document.
- If a Windows API behaves differently from what is described here, say so plainly
  rather than working around it silently. Update this file with what you learned.
- Prefer showing me a running thing over describing a plan.
