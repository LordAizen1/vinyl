# PLAN.md — vinyl

Phased build plan. Read `CLAUDE.md` first for stack, architecture, constraints, and
design direction.

**Rule: stop at each gate and wait for review.** Do not begin the next phase
unprompted. Each phase has explicit acceptance criteria; demonstrate them.

---

## Phase 0 — Spike, and the decision to continue

**This phase is throwaway code in a `spike/` directory that gets deleted in Phase 1.**
Its only job is to answer whether the data quality across real apps is good enough to
justify the project.

Build a Rust console binary that prints, once per second:

```
[app id] status | "title" — artist | pos/dur | art: yes/no | thumb bytes
```

Enumerate **all** sessions, not just the current one, so we can see what happens with
two things playing at once.

APIs:
- `windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager::RequestAsync()`
- Per session: `TryGetMediaPropertiesAsync()`, `GetPlaybackInfo()`, `GetTimelineProperties()`
- Thumbnail: `MediaProperties.Thumbnail()` → `OpenReadAsync()` → `DataReader` → bytes
- Required `windows` crate features include `Media_Control`, `Foundation`,
  `Storage_Streams`

Then have me run it for ten minutes across: Spotify, a YouTube tab in Chrome or Edge,
YouTube Music, VLC, a podcast app, a livestream, two tabs playing simultaneously, and
a game with music.

**Deliverable: a written findings table** in `spike/FINDINGS.md` recording, per app:
does the title parse usefully; is artist meaningful or is it the channel name; is
`EndTime` ever zero; does a thumbnail exist; what `GetCurrentSession()` picks when two
sources are live.

### Gate 0

Report the findings and give an explicit recommendation: proceed, proceed with a
design change, or stop. Name any app whose data is bad enough to need special-casing.
Wait for my go-ahead.

---

## Phase 1 — Ugly but alive

Port `reconcile`, `ticks_since_1601`, and their unit tests out of `spike/src/main.rs`
into `src/smtc.rs` **before** deleting `spike/`; that logic is the validated fix for
`CLAUDE.md` constraint 1 and should not be rewritten from scratch. Then delete `spike/`
and scaffold the real project: `cargo tauri init` or the Tauri v2 CLI,
vanilla frontend, no framework.

Build the real backend structure now, even though the UI is plain text:

- `src/smtc.rs` — a worker thread that owns the session manager and subscribes to
  `MediaPropertiesChanged`, `PlaybackInfoChanged`, `TimelinePropertiesChanged`,
  `CurrentSessionChanged`, and `SessionsChanged`. **Event-driven, not polled.**
  Implements the session-selection policy from `CLAUDE.md` constraint 5 rather than
  trusting `GetCurrentSession()`.
- `src/state.rs` — the `PlaybackState` struct from `CLAUDE.md` behind an `RwLock`.
- `src/lib.rs` — Tauri setup, one command `get_state()`, one emitted event
  `playback-changed`.

Frontend: a normal window with decorations and a title bar, showing the state fields
as plain text, updating live.

### Acceptance criteria

Spotify was not available during Phase 0, so demonstrate these against a YouTube tab in
Edge until Spotify is tested, then re-verify.

- Window shows correct title and artist within ~500 ms of changing track.
- Play/pause in the source app is reflected in the status text.
- Position text advances smoothly every 250 ms using the extrapolation from
  `CLAUDE.md` constraint 1, and re-anchors without jumping backwards.
- Closing all media apps shows `NoSession` rather than stale data.
- `cargo clippy -- -D warnings` clean.

### Gate 1
Demonstrate the above. This proves the entire data pipeline. Everything after this is
presentation.

---

## Phase 2 — The turntable, in isolation

**Do not touch Rust in this phase.**

Build `design/turntable.html` — a single standalone file, opened directly in a browser,
containing the complete visual driven by a fake state object plus a debug panel of
sliders and buttons: position 0–100%, playing/paused toggle, RPM 33/45, peak level
0–1, art present/absent, title and artist text fields.

This separation is the most important structural decision in the project. It lets the
visual iterate in a hot-reloading browser tab with no Rust compile, and it means the
turntable is a pure function of state.

Build order within the phase:
1. Plinth, platter, mat. Get the proportions and materials right before anything moves.
2. Grooves — concentric rings, varied spacing. Groove band from `0.92r` to `0.36r`.
3. Label — circular, at `0.36r`, with arced type on an SVG text path.
4. Rotation — CSS keyframe, 1.8 s at 33⅓ and 1.3333 s at 45.
5. Tonearm — pivot **outside** the platter, rotating about its own pivot so the stylus
   traces an arc. Map position 0→1 to an angle sweep of roughly 0°→22°.
6. Arm shadow on the record surface, moving with the arm.
7. Specular highlight sweeping the vinyl.
8. Needle drop and lift animation. The signature element — spend real time here.
9. VU treatment driven by the peak slider. Keep it restrained.

Before writing code, propose the design in prose plus an ASCII wireframe: the palette
as named hex values, the two typefaces with the specific families you're choosing and
why, the layout, and the signature moment. Check it against the "explicitly avoid"
list in `CLAUDE.md`. Show me that plan and wait, then build.

### Acceptance criteria
- Opens standalone in a browser with zero build step.
- All sliders drive the visual correctly, including the missing-art case.
- Rotation measured at 1.80 s ± 0.02 s per revolution.
- Chrome DevTools performance panel shows the platter rotation compositor-only, with
  no layout or paint per frame.
- `prefers-reduced-motion` cuts the needle-drop swing.
- Looks like equipment, not like a sticker.

### Gate 2
Screenshots at 0%, 50%, and 95% progress, plus paused, plus no-art. This is the phase
where the project succeeds or fails aesthetically — expect me to iterate here.

---

## Phase 3 — Wire them together, and the procedural label

Serve the real state into the real visual.

- Register a URI scheme protocol in Rust (`art://{id}`) serving cached thumbnail bytes.
  Hash the bytes for `art_id` so the frontend only refetches when art actually changes.
- Decode thumbnails off the UI thread. Cache by track. Never refetch per frame.
- Handle `duration_ms` of `None` or `0`: livestreams get no tonearm progress. Park the
  arm at the outer edge and hide the timecode rather than showing `0:00 / 0:00`.

Then build the **procedural label generator** — a module that takes a seeded hash of
`artist + title` and deterministically produces a label design. This is
`CLAUDE.md` constraint 3 and it should feel like a feature.

Requirements:
- Same input always yields the same label.
- At least 5 distinct archetypes with their own ring structure, type arrangement, and
  2–3 colour palette each.
- A plausible fake catalogue number derived from the same hash.
- Long titles must not overflow the arced text path — measure and truncate.

### Acceptance criteria
- Playing a Spotify track shows its real art as the spinning label.
- Playing a local file with no art shows a procedural label; the same file always
  shows the same one.
- Switching tracks swaps the label without a flash of empty white.
- A livestream does not show a bogus progress arm.
- Idle CPU under 1% measured in Task Manager over five minutes of playback.

### Gate 3

---

## Phase 4 — Make it an actual widget

Tauri window config: `transparent: true`, `decorations: false`, `alwaysOnTop: true`,
`skipTaskbar: true`, `resizable: false`, `shadow: false`. Grant the matching
capabilities in the Tauri v2 capabilities file.

- Drag to move via a designated drag region on the plinth.
- Persist window position to `%APPDATA%\vinyl\config.json`, debounced. Restore on
  launch, clamped to a currently-connected monitor so it can't restore off-screen.
- System tray icon: show/hide, always-on-top toggle, launch at login, quit.
- **Hide when a fullscreen app is foreground.** Compare the `GetForegroundWindow()`
  rect against its monitor rect from `MonitorFromWindow` + `GetMonitorInfoW`. Poll at
  1 Hz — cheap and adequate. Without this, the widget draws over games.
- Click-through stays **off** per `CLAUDE.md` constraint 2.

### Acceptance criteria
- Sits over the desktop with no chrome, no square background, no taskbar entry.
- Survives a restart in the same position. Survives unplugging a second monitor.
- Disappears in a fullscreen game and returns on alt-tab.
- Multi-monitor and 150% display scaling both behave.

### Gate 4

---

## Phase 5 — Controls, and the point where it stops being decoration

Send transport commands back through SMTC: `TryTogglePlayPauseAsync()`,
`TrySkipNextAsync()`, `TrySkipPreviousAsync()`.

- Read `PlaybackInfo.Controls` and only show a control the session actually supports.
  A disabled skip button on a livestream is a bug.
- Clicking the record or a discreet button pauses. **The needle lifts.** The physical
  ritual is the whole point — the visual and the command must feel like one action.
- Optimistically update the UI on click, then reconcile with the real SMTC event.
- Hover reveals controls; they stay hidden otherwise so the idle widget is just an object.

### Acceptance criteria
- Clicking pause actually pauses YouTube in a browser tab, with the arm lifting.
- Skip works in Spotify and is hidden or disabled where unsupported.
- No perceptible lag between click and visual response.

### Gate 5 — **this is a shippable v1.** Do not ship before this gate.

---

## Phase 6 — Audio reactivity (optional)

Two tiers. Do tier one; ask before tier two.

**Tier 1 — peak metering.** `IMMDeviceEnumerator` → `GetDefaultAudioEndpoint(eRender,
eConsole)` → `Activate::<IAudioMeterInformation>` → `GetPeakValue()` at 30 Hz. This is
about twenty lines and gives the VU needles real movement. It also means the platter
can keep turning for audio with no SMTC session at all — a recording, an obscure
player, a random browser sound.

Handle the default device changing (headphones unplugged) by detecting the error and
re-acquiring, not by crashing.

**Tier 2 — spectrum.** WASAPI loopback capture via `AUDCLNT_STREAMFLAGS_LOOPBACK` plus
`realfft`, feeding groove density. Meaningfully more work: mix format negotiation,
buffer handling, silent-packet handling. Only worth it if tier one leaves you wanting.

### Acceptance criteria
- Needles track actual loudness with sensible ballistics, not jitter.
- Platter turns for audio that registers no media session.
- Unplugging headphones mid-playback does not crash or freeze the meter.

### Gate 6

---

## Phase 7 — Ship

- Icon at all required sizes.
- README with a **looping GIF at the top**. For a project like this the GIF drives
  adoption more than the code does — record it showing a needle drop, a track change,
  and the procedural label.
- Tauri NSIS installer plus a portable `.exe` in a GitHub release.
- `winget` manifest and a Scoop bucket entry. There is no `npx` equivalent on Windows,
  so these two are the closest thing to zero-friction install.
- README must state plainly: unsigned binary, so SmartScreen will warn on first run,
  and here is why. Being upfront converts better than letting people discover it.
- `--version`, and a documented uninstall that removes the config directory.

### Acceptance criteria
- Installs and runs on a clean Windows 11 VM with no toolchain present.
- Runs on Windows 10 1809 or a stated minimum, verified not assumed.
- Total download under 15 MB.

---

## Deferred — do not build without asking

- Click-through via cursor polling
- 45 RPM as anything more than a manual toggle
- Themes and skins
- Lyrics
- Scrobbling
- macOS or Linux ports
- Any AI or LLM feature
