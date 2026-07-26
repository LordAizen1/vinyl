# Phase 0 — SMTC findings

Run the spike for at least ten minutes across the sources below. Record what the
console reports rather than what the source application's own UI displays.

## Environment

- Windows version: Windows 10 Home, 10.0.19045
- Default browser and version: Microsoft Edge 150.0.4078.83
- Test date: 2026-07-26

Spotify and YouTube Music were deliberately not tested in this pass. Everything below
comes from sources actually available on the test machine.

## Application results

| Application/source | Title useful? | Artist useful? | `EndTime` zero/unknown? | Thumbnail? | Notes |
|---|---|---|---|---|---|
| Spotify | untested | untested | untested | untested | Not tested in this pass. Deferred to a later session. |
| YouTube in Edge | Yes | Yes; reports the channel name | No in tested video | Yes | Paused session remained registered at `0:44 / 3:49`. Thumbnails observed at 17.9 KB and 22.5 KB. |
| Apple Music web player in Edge | Yes | Yes; real credited artists (`The Weeknd & Playboi Carti`), not a channel name | No; reported `4:16` | Yes; 36448 bytes, the largest observed | Best metadata quality of any browser source. `Position` read `0:00` while `playing`, which was initially alarming but turned out to be the normal Edge pattern; see the resolution below. |
| Apple Music desktop app | Yes | Needs splitting; see the packing section below | No; reported `3:19` and `2:03` | Yes; 141300 and 1022489 bytes across two tracks | **The best-behaved source tested.** Pushes a fresh timeline anchor roughly every second and its `Position` genuinely advances. While idle it still registers a session with status `opened`, empty metadata and `LastUpdatedTime` never set, which is what triggers the `GetCurrentSession()` failure below. |
| Netflix in Edge | No; only reports `Netflix` | No; blank | No | No | Playback status and timeline were correct (`14:57 / 22:36`), but the program or episode was not identified. |
| YouTube Music | untested | untested | untested | untested | Not tested in this pass. |
| VLC desktop 3.0.23 | N/A | N/A | N/A | N/A | No SMTC session appeared while VLC was actively playing. |
| Windows Photos video | No; blank | No; blank | No | No | Publishes playing status and a valid `0:36` duration, but no media identity or artwork. Position advanced in coarse snapshots (`0:06`, then `0:11`). |
| WhatsApp Web in Edge | Partial; only `WhatsApp` | No; blank | No | No | Publishes playing status and a valid `0:05` duration for an audio message. Source is `msedge.exe`. |
| WhatsApp desktop app | Partial; only `WhatsApp` | No; blank | No | No | Publishes playing status and a valid `0:05` duration. Source is the app's `msedgewebview2.exe` host. |
| Podcast app | untested | untested | untested | untested | Not tested; no podcast app installed. |
| Livestream | untested | untested | untested | untested | Not tested in this pass. The zero/unknown `EndTime` path is therefore unverified and must be handled defensively regardless. |
| Game with music | untested | untested | untested | untested | Not tested in this pass. |

## Multiple-session behavior

| Scenario | Sessions listed | `GetCurrentSession()` selected | Was that the audible/expected source? |
|---|---|---|---|
| Two Edge tabs playing simultaneously | One only; a single `msedge.exe` session | That single session, showing the *second* tab | Partly; the second tab was audible, but so was the first, and it was invisible |
| Spotify plus browser audio | untested | untested | Not tested in this pass. |
| Paused Edge plus playing VLC | Edge only; VLC absent | Paused Edge/YouTube session | No; audible VLC was invisible to SMTC |
| Playing Edge plus playing Photos | Edge and Photos | Edge/YouTube session | Ambiguous; Windows continued to prefer Edge while Photos was newly active |
| Paused Edge/YouTube plus playing WhatsApp desktop | Edge and Edge WebView2 | WhatsApp desktop WebView2 session | Yes |
| Idle Apple Music app plus playing Edge tab | Edge and Apple Music | Apple Music, which was idle and completely blank | **No.** Windows chose an empty session over audible playback, then flipped back to Edge unprompted |

### Edge collapses concurrent tabs into one session

The most consequential result of Phase 0. A song was started in one Edge tab and left
playing; a second video was then started in a new tab. Both remained audible. The spike
enumerates every session, not just the current one, and it still listed exactly one:

```
--- current: msedge.exe | "Better off Alone x Clarity x Unlock it x Alone" ---
[msedge.exe] playing | "Better off Alone x Clarity x Unlock it x Alone" — Sharingan - Topic | 2:26/3:34 | art: yes | 17943 bytes

--- current: msedge.exe | "Family Guy - My hiccups are gone" ---
[msedge.exe] playing | "Family Guy - My hiccups are gone" — Mr. Rupert | 0:00/0:58 | art: yes | 22540 bytes
```

Edge publishes a single SMTC session for the whole browser and repoints its metadata at
whichever tab it currently considers the active one. The first tab's audio kept playing
with no session of its own representing it. Enumerating all sessions therefore does not
help here: there is genuinely nothing else to enumerate.

Implication for the widget: it can only ever show Edge's chosen tab, not every
simultaneously audible tab. That is acceptable, since one record on one turntable is the
correct model, but it means a user playing two tabs will see the widget follow the tab
Edge picked, which may not be the one they are listening to. Do not attempt to work
around this; there is no SMTC surface exposing per-tab sessions.

Caveat on method: this was captured as two separate `--once` snapshots rather than one
continuous run spanning the moment the second tab started. The single-session result is
consistent across both samples and matches Chromium's known behaviour, but a brief
second session appearing and being torn down between snapshots was not ruled out. Worth
one continuous `cargo run` across a tab switch if it ever matters.

### `GetCurrentSession()` picks an idle app over a playing one

The clearest failure of Windows' own session choice, and the reason constraint 5 exists.
With the Apple Music desktop app merely installed and idle, and a YouTube tab actively
playing in Edge:

```
--- current: AppleMusic.exe | "" ---
[msedge.exe] playing | "我對緣分小心翼翼 (劇集《逐玉》主題曲)" — JJ Lin - Topic | 0:00/4:42 | anchor 272.2s ago, extrapolated 4:32 | art: yes | 15545 bytes
[AppleMusic.exe] opened | "" —  | 0:00/0:00 | anchor: never set | art: no | 0 bytes
```

Windows selected the Apple Music session, which had status `opened`, an empty title and
artist, a zero duration, no artwork, and a `LastUpdatedTime` that was never set. A
widget rendering `GetCurrentSession()` would have shown a blank record while music was
plainly audible. Note also that the selection flipped back to Edge partway through the
same run with no user action, so the choice is not even stable.

An idle installed app is enough to trigger this. It is not an exotic case.

The status enum is the discriminator: `opened` means registered but not playing, and
must rank below `playing` and `paused`.

### Resolved: the `0:00` position is normal, and extrapolation recovers it

Apple Music web reporting `0:00/4:16` while playing looked like a broken source. It is
instead the standard Edge pattern, confirmed directly on a YouTube track:

```
0:00/4:42 | anchor 272.2s ago, extrapolated 4:32
0:00/4:42 | anchor 277.2s ago, extrapolated 4:37
```

Edge publishes `position 0:00` once when a track starts and then never updates the
timeline again. The anchor was over four minutes stale, and extrapolation reconstructed
`4:32` on a `4:42` track, matching the real elapsed playback. This is case 1 below.
Raw `Position` is stuck by design; the extrapolated value is correct.

Caveat: Apple Music *web* was not itself re-measured with the instrumented build. The
mechanism was confirmed on another Edge-hosted source, and both share Edge's media
stack, so the same behaviour is very likely but not directly observed.

### Original open question: does Apple Music web advance its position at all?

Apple Music web player gave the cleanest metadata of anything tested, but reported
`0:00/4:16` while `playing` in four consecutive samples across two separate runs of the
spike:

```
--- current: msedge.exe | "Timeless" ---
[msedge.exe] playing | "Timeless" — The Weeknd & Playboi Carti | 0:00/4:16 | art: yes | 36448 bytes
```

Two possibilities, with opposite consequences for the tonearm:

1. `Position` is pinned at zero but `LastUpdatedTime` goes stale. Extrapolation from
   `CLAUDE.md` constraint 1 then produces correct smooth progress and nothing is wrong.
2. `LastUpdatedTime` refreshes on every poll while `Position` stays zero. Extrapolation
   yields roughly zero forever, and the arm never leaves the outer groove.

The spike now prints the anchor age and the extrapolated position so these can be told
apart. Growing age with growing extrapolation means case 1. Age resetting to near zero
every second while extrapolation stays at `0:00` means case 2, and Apple Music web needs
special handling or an accepted parked arm.

**Answered: case 1.** See the resolution section above. No special handling needed.

### The local clock, and why the spike now prints `reckoned`

Because case 2 would have been fatal to the tonearm, the spike now carries the fix that
handles both cases, so Phase 1 can lift it rather than rediscover it. The `reckoned`
field is a locally maintained playback clock.

The rule: **when a source republishes a position identical to the one already held, do
not re-anchor.** Keep counting locally. Re-anchor only on a track change, a genuinely
different position, or a seek, and when re-anchoring, credit the anchor's staleness so
the value stays accurate for case 1 sources.

Covered by unit tests in `spike/src/main.rs`, all passing:

- a source pinned at `0:00` forever still produces advancing progress
- a seek re-anchors to the new position instead of being smoothed away
- a paused session does not creep forward
- pausing freezes at accumulated progress, not at the stale snapshot

Verified live: a paused Edge session held steady at `1:33:12` across six samples.

### Two classes of source, and both must be served

Apple Music desktop is the first well-behaved source measured, and it is nothing like
Edge:

```
[AppleMusic.exe] playing | "Is There Someone Else?" — The Weeknd — Dawn FM | 0:18/3:19 | anchor 0.0s ago, extrapolated 0:18 | reckoned 0:18 | art: yes | 141300 bytes
[AppleMusic.exe] playing | "Is There Someone Else?" — The Weeknd — Dawn FM | 0:20/3:19 | anchor 0.1s ago, extrapolated 0:20 | reckoned 0:20 | art: yes | 141300 bytes
```

| | Edge / Chromium | Apple Music desktop |
|---|---|---|
| Anchor age | 26 s to 272 s stale | 0.0 s to 0.3 s, refreshed every sample |
| `Position` | frozen at the snapshot | genuinely advances |
| Artwork | 15–36 KB | 141300 bytes |
| Artist | channel name (`JJ Lin - Topic`) | real credited artist |

Phase 1 must serve both without special-casing either. The extrapolation is what makes
Edge move at all; Apple Music barely needs it. The same code path handles both because
a fresh anchor simply contributes a near-zero elapsed term.

`GetCurrentSession()` selected Apple Music correctly here, while it was playing. That
narrows its earlier failure specifically to **idle** sessions rather than making it
unreliable in general, though the selection policy is still required.

### Apple Music packs the album into the artist field, with no clean alternative

The `Artist` property came back as `The Weeknd — Dawn FM`, which is the artist and the
album joined with an em dash rather than the artist alone.

Confirmed on a second track that there is **no clean field to fall back to**:

```
title="Abyss (from Kaiju No. 8)"
artist="YUNGBLUD — Abyss (from Kaiju No. 8) - Single"
albumArtist="YUNGBLUD — Abyss (from Kaiju No. 8) - Single"
album=""
```

`AlbumTitle` is empty and `AlbumArtist` repeats the same packed string verbatim. So
splitting is the only route.

**Rule for Phase 1.** Split `Artist` once on the first ` — ` (space, em dash U+2014,
space). The left side is the artist; the right side is the album. Use a
split-once, not a split-all, since either side may itself contain a dash. When no
separator is present, treat the whole string as the artist, which is what every other
source produces.

Then strip Apple's ` - Single` and ` - EP` suffixes from the album. Note that for a
single, the album is just the title plus that suffix, so it carries no information and
is better dropped than displayed.

This matters directly for the arced label lettering in Phase 2: rendering the raw string
puts an album name where the artist belongs and roughly doubles the length on a text
path that has to fit.

### `reckoned` can lead `extrapolated` by up to one second

Observed on Apple Music: `extrapolated 0:18` alongside `reckoned 0:19`.

Expected, and not a defect. The local clock only re-anchors when the reported position
changes, so between changes it accumulates sub-second quantisation error against a
truncating display. It self-corrects at the next position change and the error is
bounded by the source's update interval. One second on a tonearm sweeping 22 degrees
across a three-minute track is far below the visible threshold.

### Artwork can exceed one megabyte

Thumbnail sizes observed, same session type, three different tracks:

| Source | Bytes |
|---|---|
| Edge / YouTube | 15545 to 36448 |
| Apple Music, one track | 141300 |
| Apple Music, another track | **1022489** |

A megabyte, and it varies by nearly 70x between tracks from the same app. The spike
re-reads the thumbnail on every one-second pass by design, so it is currently moving
about 1 MB/s across the COM boundary for a single session.

The real widget must not do this. `PLAN.md` Phase 3 already says to cache by track and
never refetch per frame; this finding sets the actual stakes. Concretely:

- Re-read the thumbnail **only when the track identity changes**, never on a timer.
  Hashing the bytes for `art_id` requires reading them, so the read itself must be
  gated on identity, not used to detect change.
- Decode off the UI thread, as already specified.
- The idle CPU budget in `CLAUDE.md` constraint 4 is under 1%. A per-second megabyte
  read and decode would not fit inside it.

### Possible stale thumbnail in Edge, unconfirmed

The Edge session reported a thumbnail of exactly `15545` bytes for a JJ Lin music video
and then, after the session metadata swapped to an unrelated film page titled
`Watch Soulmate`, reported `15545` bytes again. Two unrelated items producing a
byte-identical thumbnail size is unlikely to be coincidence, and suggests Edge can leave
a stale thumbnail attached after its metadata changes.

Not confirmed; the bytes themselves were not compared. It matters for Phase 3, where
`art_id` is a hash of the thumbnail bytes: if the bytes go stale, the widget will
confidently display the previous track's artwork. Worth comparing hashes across a track
change before relying on it.

## Timeline observations

**Does `Position` advance, update intermittently, or remain frozen?** Frozen, and the
anchor timestamp is what moves. Measured on YouTube in Edge with three consecutive
one-second samples:

```
[msedge.exe] playing | ... | 2:38/4:12 | anchor 24.9s ago, extrapolated 3:02 | art: yes
[msedge.exe] playing | ... | 2:38/4:12 | anchor 25.9s ago, extrapolated 3:03 | art: yes
[msedge.exe] playing | ... | 2:38/4:12 | anchor 26.9s ago, extrapolated 3:04 | art: yes
```

`Position` never left `2:38`. The anchor age grew by exactly 1.0 s per sample, so
`LastUpdatedTime` is a **fixed anchor that goes stale**, not a timestamp the source
refreshes on every poll. Edge had not pushed a timeline update in over 26 seconds.

**This empirically validates `CLAUDE.md` constraint 1.** `position + (now - updated_at)`
reconstructs smooth, correct progress from a snapshot that is nearly half a minute old.
The extrapolation is not a workaround for a rare case; it is the only thing that makes
the tonearm move at all for browser media. A widget that renders `Position` directly
would have shown `2:38` frozen for the entire track.

Practical consequence for Phase 1: `updated_at` must be the source's `LastUpdatedTime`
converted to the local clock, **not** the time we happened to poll. Anchoring to poll
time would restart the extrapolation from zero on every read and freeze the arm.

- Does seeking produce an immediate timeline update? Still unmeasured. Worth one run
  with a deliberate scrub, since a seek that does *not* push a fresh anchor would leave
  the extrapolation confidently wrong until the next natural update.
- Do position or duration values jump backwards? Not observed, but not deliberately
  exercised either. Note that with anchors this stale, a correction arriving 26 seconds
  late could be a large one, so the "interpolate corrections under 1500 ms, never jump
  backwards" rule in constraint 1 matters more than it first appeared.

## Recommendation

**Proceed with a design change.**

The data is good enough. For the two sources that matter most to this widget (a YouTube
tab, and Chromium-hosted media generally) title, artist, timeline, and artwork all
arrive usefully, and artwork is present at a usable size. The failure modes found are
all in the direction of *missing* metadata rather than *wrong* metadata, which the
procedural label in `CLAUDE.md` constraint 3 already turns into a feature. Nothing
found here undermines the project.

Three design changes fall out of the results:

1. **Session selection needs an explicit policy.** `GetCurrentSession()` kept choosing
   Edge while Photos was also `Playing`, and the paused Edge session stayed registered
   indefinitely. Phase 1 must pick a session itself, preferring `Playing` over `Paused`
   and breaking ties by most recently updated, rather than trusting Windows' choice.
2. **The procedural label seed must not be `artist + title` alone.** Photos, Netflix,
   and WhatsApp all report blank or generic identity, so every one of them would hash
   to the same label. Fold the source app into the seed, and for WhatsApp desktop use
   the reported title rather than the process name, since it hosts as the shared
   `msedgewebview2.exe`.
3. **The widget follows one session, and that is now a documented limitation**, not an
   oversight. See the Edge tab-collapse section above.
4. **Artist strings must be sanitised before they reach the label.** Apple Music packs
   `Artist — Album` into `Artist` with no clean field to fall back on, and YouTube
   reports channel names like `JJ Lin - Topic`.
5. **Thumbnail reads must be gated on track identity, not polled.** Art was measured at
   over a megabyte, so reading it per tick to detect change would blow the idle CPU
   budget on its own.

Apps requiring special handling:

- VLC desktop did not publish an SMTC session in the tested installation. The widget
  cannot obtain its metadata, artwork, timeline, or controls through SMTC. Phase 6
  peak metering could detect that audio exists, but not identify the media.
- Netflix in Edge publishes status and timeline but only identifies the item as
  `Netflix`, with no artist or artwork. Without a browser extension, the widget must
  display a generic Netflix label rather than the actual program or episode.
- Windows Photos publishes a usable timeline but blank title, artist, and artwork.
  Its procedural label seed must include the source app so empty metadata still has
  a deterministic, recognizable generic label.
- `GetCurrentSession()` continued to select Edge when Edge and Photos both reported
  `Playing`. The real application needs an explicit session-selection policy rather
  than treating Windows' current session as authoritative.
- WhatsApp Web and desktop both expose status and duration but only generic
  `WhatsApp` metadata and no artwork. The desktop app reports the shared
  `msedgewebview2.exe` host, so its title, not the process name, must drive the generic
  procedural label.
- Edge, and by extension every Chromium browser, publishes one session for all tabs.
  Two tabs playing at once are representable only as whichever one Edge selected.

Untested and carrying risk into later phases:

- Spotify, YouTube Music, podcasts, and games were not exercised. Spotify in particular
  is what Phase 1's acceptance criteria are written around, so those criteria are
  currently unverifiable as written and should be demonstrated against a YouTube tab
  until Spotify is tested.
- No livestream was tested, so a zero or unknown `EndTime` has never actually been
  seen. `PLAN.md` Phase 3 requires handling it; implement that from the spec rather
  than from observation.
