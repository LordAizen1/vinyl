//! Timed lyrics, from LRCLIB.
//!
//! SMTC carries no lyrics field of any kind, so this is the one thing the widget
//! cannot read off the machine. LRCLIB is a free, key-less, community LRC
//! database and the only outbound request the app makes: a track's title,
//! artist, album and duration, and nothing else, when the track changes.
//!
//! Timed, not plain. The widget already extrapolates playback position
//! accurately (see `smtc.rs`), so `[mm:ss.xx]` stamps are what make the scroll
//! track the song rather than merely display words.

use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::state::PlaybackState;

const ENDPOINT: &str = "https://lrclib.net/api";
const TIMEOUT: Duration = Duration::from_secs(8);

/// LRCLIB asks that clients identify themselves.
const AGENT: &str = concat!(
    "vinyl/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/LordAizen1/vinyl)"
);

/// One line, and the moment it starts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub at_ms: u64,
    pub text: String,
}

/// What the frontend receives for the current track.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// The track these belong to, so a reply that arrives after the user has
    /// skipped on can be discarded rather than shown against the wrong song.
    pub track_key: String,
    pub lines: Vec<Line>,
    /// LRCLIB knows the track and says it has no words. Worth distinguishing
    /// from "not found": one is an answer, the other is a miss.
    pub instrumental: bool,
}

/// What LRCLIB returns from `/api/get` and each element of `/api/search`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Track {
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    synced_lyrics: Option<String>,
}

/// Identifies a track well enough to look it up and to spot a stale reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
}

impl Query {
    /// A stable key for "is this still the track we are showing".
    pub fn key(&self) -> String {
        format!("{}|{}", self.artist, self.title)
    }
}

/// Strips the decoration browsers pack into a title.
///
/// Phase 0 found that a title from Edge arrives as the whole video name, so
/// "The Weeknd - Alone Again (Official Audio)" rather than "Alone Again". The
/// exact endpoint 404s on those; measured, the search endpoint still finds them,
/// but only once the noise is gone. Sources with real metadata (the Apple Music
/// and Spotify apps) are unaffected, because there is nothing here to strip.
fn clean(title: &str) -> String {
    const NOISE: [&str; 12] = [
        "official video",
        "official audio",
        "official music video",
        "official lyric video",
        "lyric video",
        "lyrics",
        "audio",
        "visualizer",
        "hd",
        "4k",
        "remastered",
        "explicit",
    ];

    let mut out = String::with_capacity(title.len());
    let mut depth = 0usize;
    let mut group = String::new();

    // Bracketed groups are dropped only when they are noise: "(Deluxe)" and
    // "(feat. Ariana Grande)" change which track this is and must survive.
    for ch in title.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                group.clear();
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                let lower = group.trim().to_lowercase();
                if !NOISE.iter().any(|n| lower == *n) {
                    out.push('(');
                    out.push_str(group.trim());
                    out.push(')');
                }
                group.clear();
            }
            _ if depth > 0 => group.push(ch),
            _ => out.push(ch),
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses LRC into ascending lines.
///
/// Handles what LRCLIB actually returns: `[ar:]`-style metadata headers, blank
/// lines standing for instrumental gaps, and one line carrying several stamps
/// (a repeated chorus is stored once). Anything unparseable is skipped rather
/// than failing the whole fetch, since one malformed stamp should not cost the
/// other thirty lines.
pub fn parse_lrc(body: &str) -> Vec<Line> {
    let mut lines = Vec::new();

    for raw in body.lines() {
        let mut rest = raw;
        let mut stamps = Vec::new();

        // Consume every leading [..] group, then whatever is left is the text.
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            let inside = &rest[1..close];
            if let Some(ms) = parse_stamp(inside) {
                stamps.push(ms);
            } else if !inside.contains(':') {
                // Neither a stamp nor a metadata header; stop rather than eat
                // something that might be the text itself.
                break;
            }
            rest = &rest[close + 1..];
        }

        if stamps.is_empty() {
            continue;
        }

        let text = rest.trim().to_owned();
        for at_ms in stamps {
            lines.push(Line {
                at_ms,
                text: text.clone(),
            });
        }
    }

    lines.sort_by_key(|line| line.at_ms);
    lines
}

/// `mm:ss.xx`, `mm:ss.xxx` or `mm:ss`. Returns `None` for metadata like `ar:x`.
fn parse_stamp(inside: &str) -> Option<u64> {
    let (minutes, tail) = inside.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;

    let (seconds, fraction) = match tail.split_once(['.', ':']) {
        Some((s, f)) => (s, f),
        None => (tail, ""),
    };

    let seconds: u64 = seconds.trim().parse().ok()?;
    if seconds >= 60 {
        return None;
    }

    // Two digits is centiseconds, three is milliseconds.
    let fraction_ms = match fraction.len() {
        0 => 0,
        2 => fraction.parse::<u64>().ok()? * 10,
        3 => fraction.parse::<u64>().ok()?,
        _ => return None,
    };

    Some(minutes * 60_000 + seconds * 1_000 + fraction_ms)
}

/// Looks a track up, exact first and fuzzy second.
///
/// Returns `None` when nothing matches, which is the common case for podcasts,
/// game audio and anything a browser reports oddly. That is not an error and is
/// not logged as one.
pub fn fetch(query: &Query) -> Option<Lyrics> {
    let title = clean(&query.title);
    if title.is_empty() {
        return None;
    }

    let key = query.key();

    if let Some(track) = get_exact(&title, query) {
        return Some(into_lyrics(track, key));
    }

    // The exact endpoint wants the album and duration to agree. Browsers supply
    // neither reliably, so fall back to a plain text search.
    let terms = if query.artist.is_empty() {
        title.clone()
    } else {
        format!("{} {}", query.artist, title)
    };

    search(&terms).map(|track| into_lyrics(track, key))
}

fn into_lyrics(track: Track, track_key: String) -> Lyrics {
    Lyrics {
        track_key,
        lines: track
            .synced_lyrics
            .as_deref()
            .map(parse_lrc)
            .unwrap_or_default(),
        instrumental: track.instrumental,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(AGENT)
        .build()
        .into()
}

fn get_exact(title: &str, query: &Query) -> Option<Track> {
    let mut url = format!(
        "{ENDPOINT}/get?track_name={}&artist_name={}",
        encode(title),
        encode(&query.artist)
    );
    if !query.album.is_empty() {
        url.push_str(&format!("&album_name={}", encode(&query.album)));
    }
    if let Some(ms) = query.duration_ms {
        url.push_str(&format!("&duration={}", ms / 1000));
    }

    let track: Track = agent().get(&url).call().ok()?.body_mut().read_json().ok()?;
    has_words(track)
}

fn search(terms: &str) -> Option<Track> {
    let url = format!("{ENDPOINT}/search?q={}", encode(terms));
    let found: Vec<Track> = agent().get(&url).call().ok()?.body_mut().read_json().ok()?;

    // The first hit that actually carries timed lyrics. Search is ranked by
    // relevance, and plenty of entries hold only plain text.
    found.into_iter().find_map(has_words)
}

/// Timed lyrics, or a definite "this track is an instrumental". A hit with only
/// plain lyrics is no use here: without stamps there is nothing to scroll.
fn has_words(track: Track) -> Option<Track> {
    let usable = track.instrumental
        || track
            .synced_lyrics
            .as_deref()
            .is_some_and(|body| !body.trim().is_empty());

    usable.then_some(track)
}

/// Percent-encodes a query parameter.
///
/// Hand-rolled to avoid a dependency for one function. Everything outside the
/// unreserved set of RFC 3986 is escaped, which is stricter than necessary but
/// never wrong.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/* ══════════════════════════════════════════════════════════════════════
 * The worker
 * ══════════════════════════════════════════════════════════════════════ */

/// Emitted when the lyrics for the showing track change, including to nothing.
pub const CHANGED_EVENT: &str = "lyrics-changed";

/// How many tracks of lyrics to keep. Skipping back a track should not refetch,
/// but this is a widget and the words of eight songs is already generous.
const MAX_CACHED: usize = 8;

/// The last thing emitted, so a frontend that connects late can catch up.
///
/// Without this the words were event-only, and Rust reaches a track long before
/// the webview finishes loading: the first emit landed with nobody listening
/// and nothing ever asked again, so the song playing at launch never got its
/// lyrics. Every other piece of state has an initial read for exactly this
/// reason (`get_state`, `get_prefs`); this is the one for lyrics.
pub type SharedLyrics = Arc<RwLock<Lyrics>>;

pub fn shared() -> SharedLyrics {
    Arc::new(RwLock::new(Lyrics::default()))
}

/// Runs on its own thread. Network calls block for up to `TIMEOUT`, which must
/// never be on the SMTC worker: that thread owns the WinRT session subscriptions
/// and stalling it would freeze the whole widget behind a slow lookup.
pub fn spawn(app: AppHandle, current: SharedLyrics) -> Sender<Option<Query>> {
    let (tx, rx) = mpsc::channel::<Option<Query>>();

    thread::spawn(move || {
        log::debug!("lyrics: worker started");
        // A panic here would kill only this thread. Sends would keep succeeding
        // into a channel nobody reads, and lyrics would simply never appear with
        // nothing in the log to say why. Say so instead.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut cache: Vec<(String, Lyrics)> = Vec::new();
            let mut showing = String::new();

            while let Ok(request) = rx.recv() {
                // Only the newest request matters. A burst of track changes while a
                // fetch was in flight should cost one lookup, not one per skip.
                let mut request = request;
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }

                let Some(query) = request else {
                    if !showing.is_empty() {
                        showing.clear();
                        emit(&app, &current, Lyrics::default());
                    }
                    continue;
                };

                let key = query.key();
                if key == showing {
                    continue;
                }
                log::debug!("lyrics: looking up {key:?}");

                if let Some((_, hit)) = cache.iter().find(|(cached, _)| *cached == key) {
                    showing = key;
                    emit(&app, &current, hit.clone());
                    continue;
                }

                // Clear first: the old song's words must not sit under the new
                // song's title for however long the request takes.
                showing = key.clone();
                emit(&app, &current, Lyrics::default());

                let found = fetch(&query).unwrap_or(Lyrics {
                    track_key: key.clone(),
                    ..Lyrics::default()
                });

                log::info!(
                    "lyrics: {:?} by {:?} -> {}",
                    query.title,
                    query.artist,
                    if found.instrumental {
                        "instrumental".to_owned()
                    } else if found.lines.is_empty() {
                        "no match".to_owned()
                    } else {
                        format!("{} lines", found.lines.len())
                    }
                );

                cache.push((key.clone(), found.clone()));
                if cache.len() > MAX_CACHED {
                    cache.remove(0);
                }

                emit(&app, &current, found);
            }
        }));

        match outcome {
            Ok(()) => log::debug!("lyrics: worker stopped, the channel closed"),
            Err(_) => log::error!(
                "lyrics: worker PANICKED and has stopped; no lyrics will appear \
                 until restart"
            ),
        }
    });

    tx
}

fn emit(app: &AppHandle, current: &SharedLyrics, lyrics: Lyrics) {
    log::debug!(
        "lyrics: emitting trackKey={:?} with {} lines",
        lyrics.track_key,
        lyrics.lines.len()
    );
    // Recorded before the emit, so a `get_lyrics` racing this always sees a
    // state at least as new as the event it might have missed.
    *current.write() = lyrics.clone();

    if let Err(error) = app.emit(CHANGED_EVENT, lyrics) {
        log::warn!("lyrics: could not emit ({error})");
    }
}

/// Builds the lookup for a snapshot, or `None` when one should not be made.
///
/// Returning `None` is the same signal as "clear the lyrics", which is why the
/// paused case is *not* here: pausing does not change the words.
pub fn query_for(state: &PlaybackState, enabled: bool) -> Option<Query> {
    if !enabled {
        log::debug!("lyrics: skipped, turned off in the menu");
        return None;
    }
    if !state.media_kind.might_have_lyrics() {
        log::debug!("lyrics: skipped, {:?} is not music", state.media_kind);
        return None;
    }

    let Some(title) = state.title.clone() else {
        log::debug!("lyrics: skipped, no title");
        return None;
    };
    // A livestream has no fixed position to sync against, so timed lines would
    // scroll against nothing.
    let Some(duration_ms) = state.duration_ms.filter(|ms| *ms > 0) else {
        log::debug!("lyrics: skipped, {title:?} reports no duration");
        return None;
    };

    Some(Query {
        title,
        artist: state.artist.clone().unwrap_or_default(),
        album: state.album.clone().unwrap_or_default(),
        duration_ms: Some(duration_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MediaKind;

    fn music(title: &str) -> PlaybackState {
        PlaybackState {
            title: Some(title.to_owned()),
            artist: Some("The Weeknd".into()),
            duration_ms: Some(250_000),
            media_kind: MediaKind::Music,
            ..PlaybackState::no_session()
        }
    }

    #[test]
    fn a_film_is_never_looked_up() {
        let mut state = music("Some Film");
        state.media_kind = MediaKind::Video;
        assert!(query_for(&state, true).is_none());
    }

    #[test]
    fn an_unknown_kind_still_is_because_browsers_leave_it_unset() {
        let mut state = music("Alone Again");
        state.media_kind = MediaKind::Unknown;
        assert!(query_for(&state, true).is_some());
    }

    #[test]
    fn a_livestream_is_not_looked_up() {
        // No duration means no position to sync timed lines against.
        let mut state = music("Lofi radio");
        state.duration_ms = None;
        assert!(query_for(&state, true).is_none());
    }

    #[test]
    fn the_toggle_stops_it_entirely() {
        assert!(query_for(&music("Alone Again"), false).is_none());
    }

    #[test]
    fn parses_the_shape_lrclib_returns() {
        let body = "[ar:The Weeknd]\n[00:11.55] Take off my disguise\n[00:17.46] I'm living someone else's life";
        let lines = parse_lrc(body);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].at_ms, 11_550);
        assert_eq!(lines[0].text, "Take off my disguise");
        assert_eq!(lines[1].at_ms, 17_460);
    }

    #[test]
    fn keeps_blank_lines_because_they_are_the_instrumental_gaps() {
        // A stamp with no words means "nothing is sung here", which the scroll
        // needs in order to clear the line rather than hold the last one.
        let lines = parse_lrc("[00:00.00]\n[00:11.55] Take off my disguise");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "");
    }

    #[test]
    fn one_line_can_carry_several_stamps() {
        let lines = parse_lrc("[00:10.00][01:20.50] Together we're alone");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].at_ms, 10_000);
        assert_eq!(lines[1].at_ms, 80_500);
        assert_eq!(lines[1].text, "Together we're alone");
    }

    #[test]
    fn comes_back_sorted_even_when_the_file_is_not() {
        let lines = parse_lrc("[00:30.00] later\n[00:10.00] earlier");
        assert_eq!(lines[0].text, "earlier");
        assert_eq!(lines[1].text, "later");
    }

    #[test]
    fn milliseconds_and_centiseconds_both_work() {
        assert_eq!(parse_stamp("00:11.55"), Some(11_550));
        assert_eq!(parse_stamp("00:11.550"), Some(11_550));
        assert_eq!(parse_stamp("01:00"), Some(60_000));
    }

    #[test]
    fn metadata_headers_are_not_stamps() {
        assert_eq!(parse_stamp("ar:The Weeknd"), None);
        assert_eq!(parse_stamp("length:4:10"), None);
        // 61 seconds is not a real stamp; it would be 01:01.
        assert_eq!(parse_stamp("00:61.00"), None);
    }

    #[test]
    fn a_malformed_line_costs_only_itself() {
        let lines = parse_lrc("[garbage] nope\n[00:11.55] Take off my disguise");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Take off my disguise");
    }

    #[test]
    fn strips_the_decoration_a_browser_packs_into_a_title() {
        assert_eq!(
            clean("The Weeknd - Alone Again (Official Audio)"),
            "The Weeknd - Alone Again"
        );
        assert_eq!(clean("Blinding Lights [Official Video]"), "Blinding Lights");
    }

    #[test]
    fn keeps_the_brackets_that_change_which_track_this_is() {
        // Dropping these would look up the wrong recording.
        assert_eq!(clean("Save Your Tears (Remix)"), "Save Your Tears (Remix)");
        assert_eq!(
            clean("Die For You (feat. Ariana Grande)"),
            "Die For You (feat. Ariana Grande)"
        );
    }

    #[test]
    fn key_is_stable_for_the_same_track() {
        let query = Query {
            title: "Alone Again".into(),
            artist: "The Weeknd".into(),
            album: "After Hours".into(),
            duration_ms: Some(250_000),
        };
        let other = Query {
            album: "After Hours (Deluxe)".into(),
            duration_ms: Some(251_000),
            ..query.clone()
        };
        // Album and duration wobble between sources; artist and title do not.
        assert_eq!(query.key(), other.key());
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Hits the real service, so it is not part of the normal run.
    /// `cargo test --lib -- --ignored --nocapture`
    #[test]
    #[ignore = "network"]
    fn fetches_a_real_track() {
        let found = fetch(&Query {
            title: "Alone Again".into(),
            artist: "The Weeknd".into(),
            album: "After Hours".into(),
            duration_ms: Some(250_000),
        })
        .expect("exact lookup should hit");
        assert!(found.lines.len() > 10, "got {} lines", found.lines.len());
        println!(
            "exact: {} lines, first {:?}",
            found.lines.len(),
            found.lines[0]
        );
    }

    /// The browser case: a whole video name, no album, no artist.
    #[test]
    #[ignore = "network"]
    fn falls_back_to_search_for_a_browser_title() {
        let found = fetch(&Query {
            title: "The Weeknd - Alone Again (Official Audio)".into(),
            artist: String::new(),
            album: String::new(),
            duration_ms: None,
        })
        .expect("search fallback should hit");
        assert!(!found.lines.is_empty());
        println!(
            "search: {} lines, first {:?}",
            found.lines.len(),
            found.lines[0]
        );
    }

    #[test]
    #[ignore = "network"]
    fn a_miss_is_none_not_an_error() {
        assert!(fetch(&Query {
            title: "zzzz not a real song zzzz".into(),
            artist: "nobody at all".into(),
            album: String::new(),
            duration_ms: None,
        })
        .is_none());
    }
}
