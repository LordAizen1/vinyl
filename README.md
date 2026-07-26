# vinyl

A little turntable that sits on your desktop and plays along with whatever you're listening to.

Put something on — Spotify, Apple Music, a YouTube tab, honestly anything — and the record starts spinning. The tonearm drifts inwards as the song goes on, the way a real one does. The album art becomes the label in the middle. If the song has lyrics, they scroll past on the little screen.

That's the whole thing, really.

## Why

I kept catching myself glancing at the corner of my screen to see what was playing, and the answer was always some tiny bit of text in a taskbar somewhere. One day it occurred to me that a record player would be a much nicer way to be told, and then I couldn't stop thinking about it. So here we are.

It's not trying to replace your music player. You can't browse anything, there's no library, no playlists. It just sits there and spins.

<!-- TODO: drop a screenshot or a gif in here -->

## Getting it

Grab the installer from [Releases](../../releases) and run it. Windows 10 or 11.

Heads up: it isn't code-signed, because certificates cost more per year than this project is worth. So Windows will show you a **"Windows protected your PC"** box the first time. Click **More info**, then **Run anyway**. Entirely up to you whether you trust a stranger's turntable, and completely fair if you'd rather build it yourself from source.

## Using it

It lives on your desktop, underneath your other windows. Open something and it disappears behind it; go back to your desktop and there it is again.

- **Drag it** from anywhere on its body. It won't let you shove it off the edge of the screen or under the taskbar.
- **Right-click it** (or the tray icon) for the menu.
- **Left-click the tray icon** to hide or show it.

In the menu you'll find:

| | |
|---|---|
| **Full size / Compact** | Compact is just the deck, nothing else. Nice if you want it out of the way. |
| **Appearance** | Light, dark, or follow whatever Windows is doing. |
| **Show lyrics** | On by default. See the note below. |
| **Quit vinyl** | Does what it says. |

## The lyrics thing

Windows tells the app what's playing, but it has no idea about lyrics, nobody does. So when a song starts, it asks [LRCLIB](https://lrclib.net) — a free, community-run lyrics database — whether it knows that one.

Which means: **the song title and artist get sent to lrclib.net.** Nothing else leaves your machine, and if you untick **Show lyrics** the app makes no internet requests at all, ever.

Fair warning, it doesn't always find them. Apps like Spotify and Apple Music report nice clean song names so those usually work. YouTube reports the whole video title, `Some Artist - Song Name (Official Video) [4K]`, which is a bit more of a guessing game. It tries its best.

## Building it yourself

You'll want [Rust](https://rustup.rs) and [Node](https://nodejs.org).

```bash
npm install
npm run tauri dev     # run it
npm run tauri build   # make an installer
```

The installer lands in `src-tauri/target/release/bundle/`.

If you fancy poking at the look of it without waiting for Rust to compile every time, there's a browser harness:

```bash
python -m http.server 8731
# then open http://127.0.0.1:8731/design/app-preview.html
```

It runs the real interface against fake data. Add `?state=none`, `?size=compact`, `?theme=dark`, that sort of thing. The options are listed in a comment at the top of the file.

## If something breaks

There's a log at `%LOCALAPPDATA%\vinyl\vinyl.log`, wiped fresh each time it starts. It usually says something useful. Feel free to open an issue and paste it in.

Your settings live in `%APPDATA%\dev.lordaizen.vinyl\config.json` — window position, size, theme, whether lyrics are on. Delete it and everything goes back to defaults.

## Bits and pieces

Built with [Tauri](https://tauri.app), so it's a Rust backend with an HTML front end, and the whole thing is under 50 MB of memory. The turntable is hand-drawn SVG, no images. It reads Windows' own media session (SMTC), which is the same thing that powers the little popup when you press the volume keys, so it works with anything that talks to Windows properly.

Fonts and icons belong to other people and their licences are in [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).

MIT licensed. Do what you like with it.
