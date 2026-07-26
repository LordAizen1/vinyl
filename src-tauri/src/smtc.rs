//! The SMTC worker thread: owns the session manager, subscribes to change
//! events, and keeps `PlaybackState` current.
//!
//! Three pieces of hard-won behaviour live here, all measured in Phase 0 and
//! documented in `docs/FINDINGS.md`:
//!
//! 1. `Position` does not tick. It is a snapshot as of `LastUpdatedTime`, and
//!    Edge routinely leaves one untouched for minutes. We publish the anchor and
//!    let the frontend extrapolate.
//! 2. When a source republishes a position identical to the one we already hold,
//!    we must not move the anchor forward, or a stuck source pins at `0:00`.
//! 3. `GetCurrentSession()` is not authoritative. It picked an idle, blank Apple
//!    Music session over an audible YouTube tab, so we rank sessions ourselves.

use std::collections::HashMap;
use std::future::{Future, IntoFuture};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tauri::{AppHandle, Emitter};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as SmtcStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::art::ArtCache;
use crate::state::{PlaybackState, SharedState, Status};

/// Safety net only. Events drive the updates; this bounds how long a missed
/// event can leave the UI stale. Reading metadata is cheap because Phase 1 does
/// not touch thumbnails, which are the expensive part (measured at 1 MB).
const WATCHDOG: Duration = Duration::from_secs(5);

/// Unix epoch expressed in the 100 ns ticks since 1601 that WinRT `DateTime` uses.
const UNIX_EPOCH_IN_1601_TICKS: i64 = 116_444_736_000_000_000;

/// Starts the worker. Never blocks the caller.
pub fn spawn(app: AppHandle, state: SharedState, art: Arc<ArtCache>) {
    thread::spawn(move || {
        if let Err(error) = run(&app, &state, &art) {
            log::error!("SMTC worker stopped: {error:#}");
        }
    });
}

fn run(app: &AppHandle, state: &SharedState, art: &ArtCache) -> Result<()> {
    // SAFETY: first WinRT call on this thread, and every WinRT call made by this
    // worker stays on it. MTA means event callbacks arrive on pool threads
    // without needing a message pump.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };

    let manager = block_on(GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?)?;
    let (tx, rx) = mpsc::channel::<()>();

    let sessions_tx = tx.clone();
    manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = sessions_tx.send(());
        Ok(())
    }))?;

    let current_tx = tx.clone();
    manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = current_tx.send(());
        Ok(())
    }))?;

    let mut subscriptions = Subscriptions::default();
    let mut anchors: HashMap<String, Anchor> = HashMap::new();
    let mut artwork = ArtTracker::default();

    loop {
        if let Err(error) = subscriptions.sync(&manager, &tx) {
            log::warn!("could not refresh session subscriptions: {error:#}");
        }

        match read(&manager, &mut anchors, &mut artwork, art) {
            Ok(snapshot) => publish(app, state, snapshot),
            Err(error) => log::warn!("SMTC read failed: {error:#}"),
        }

        // Wait for an event, or fall through on the watchdog.
        let _ = rx.recv_timeout(WATCHDOG);
        // Coalesce a burst: sources fire several change events per track change.
        while rx.try_recv().is_ok() {}
    }
}

fn publish(app: &AppHandle, state: &SharedState, snapshot: PlaybackState) {
    {
        let current = state.read();
        if !current.differs_from(&snapshot) {
            return;
        }
    }

    *state.write() = snapshot.clone();

    if let Err(error) = app.emit("playback-changed", snapshot) {
        log::warn!("could not emit playback-changed: {error}");
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

fn read(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
    anchors: &mut HashMap<String, Anchor>,
    artwork: &mut ArtTracker,
    art: &ArtCache,
) -> Result<PlaybackState> {
    let sessions = manager.GetSessions()?;

    let mut best: Option<(u8, i64, GlobalSystemMediaTransportControlsSession)> = None;

    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;

        let Ok(playback) = session.GetPlaybackInfo() else {
            continue;
        };
        let Ok(status) = playback.PlaybackStatus() else {
            continue;
        };

        let rank = rank_of(status);
        if rank == 0 {
            // Registered but idle. An installed-but-not-playing app must never
            // win; Windows itself gets this wrong.
            continue;
        }

        let anchor_ticks = session
            .GetTimelineProperties()
            .and_then(|timeline| timeline.LastUpdatedTime())
            .map(|updated| updated.UniversalTime)
            .unwrap_or(0);

        let better = match &best {
            None => true,
            Some((best_rank, best_ticks, _)) => {
                rank > *best_rank || (rank == *best_rank && anchor_ticks > *best_ticks)
            }
        };

        if better {
            best = Some((rank, anchor_ticks, session));
        }
    }

    let Some((_, _, session)) = best else {
        anchors.clear();
        *artwork = ArtTracker::default();
        return Ok(PlaybackState::no_session());
    };

    describe(&session, anchors, artwork, art)
}

/// Ranks a session for selection. Higher wins; zero is never selected.
fn rank_of(status: SmtcStatus) -> u8 {
    match status {
        SmtcStatus::Playing => 3,
        SmtcStatus::Paused => 2,
        SmtcStatus::Changing | SmtcStatus::Stopped => 1,
        // `Opened` means registered but idle, and `Closed` is gone.
        _ => 0,
    }
}

fn describe(
    session: &GlobalSystemMediaTransportControlsSession,
    anchors: &mut HashMap<String, Anchor>,
    artwork: &mut ArtTracker,
    art: &ArtCache,
) -> Result<PlaybackState> {
    let source_app = session.SourceAppUserModelId()?.to_string();
    let media = block_on(session.TryGetMediaPropertiesAsync()?)?;
    let playback = session.GetPlaybackInfo()?;
    let timeline = session.GetTimelineProperties()?;

    let title = non_empty(media.Title().map(|v| v.to_string()).unwrap_or_default());
    let raw_artist = media.Artist().map(|v| v.to_string()).unwrap_or_default();
    let raw_album = media
        .AlbumTitle()
        .map(|v| v.to_string())
        .unwrap_or_default();

    let (artist, album_from_artist) = split_artist(&raw_artist);
    let album = non_empty(clean_album(&raw_album)).or(album_from_artist);

    let status = match playback.PlaybackStatus()? {
        SmtcStatus::Playing => Status::Playing,
        SmtcStatus::Paused => Status::Paused,
        _ => Status::Stopped,
    };

    let position_ms = ticks_to_ms(timeline.Position()?.Duration);
    let duration_ms = ticks_to_ms(timeline.EndTime()?.Duration);
    let anchor_ticks = timeline.LastUpdatedTime()?.UniversalTime;

    let identity = format!("{}|{}", title.as_deref().unwrap_or(""), raw_artist);
    let anchor = anchors
        .entry(source_app.clone())
        .or_insert_with(|| Anchor::new(&identity, position_ms, anchor_ticks));

    anchor.observe(&identity, position_ms, anchor_ticks);

    // Reading the thumbnail is the expensive part of this whole module, so it
    // is gated on the track identity changing rather than run on every wake.
    // Phase 0 measured one at 1,022,489 bytes; doing this per tick would not
    // fit the idle CPU budget in CLAUDE.md constraint 4.
    let art_key = format!("{source_app}|{identity}");
    if artwork.identity != art_key {
        artwork.identity = art_key;
        artwork.art_id = read_thumbnail(&media).map(|bytes| art.insert(bytes));
    }

    Ok(PlaybackState {
        title,
        artist,
        album,
        art_id: artwork.art_id.clone(),
        status,
        position_ms: Some(anchor.published_position_ms),
        duration_ms: (duration_ms > 0).then_some(duration_ms),
        updated_at: anchor.published_updated_at,
        source_app,
        peak: 0.0,
    })
}

// ---------------------------------------------------------------------------
// Artwork
// ---------------------------------------------------------------------------

/// Remembers which track's art we already hold, so the thumbnail is read once
/// per track rather than once per event.
#[derive(Default)]
struct ArtTracker {
    identity: String,
    art_id: Option<String>,
}

/// Pulls the thumbnail bytes out of a session.
///
/// Every step is fallible and every failure is simply "no art", which is the
/// common case: Phase 0 found browsers, local files and livestreams routinely
/// have no thumbnail at all. That is what the procedural label is for.
fn read_thumbnail(media: &MediaProperties) -> Option<Vec<u8>> {
    let reference = media.Thumbnail().ok()?;
    let stream = block_on(reference.OpenReadAsync().ok()?).ok()?;

    let size = stream.Size().ok()?;
    if size == 0 {
        return None;
    }

    let readable = u32::try_from(size).ok()?;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = block_on(reader.LoadAsync(readable).ok()?).ok()?;

    let mut bytes = vec![0_u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;

    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

// ---------------------------------------------------------------------------
// The anchor: constraint 1, and the rule that a republished position must not
// move it. Ported from the Phase 0 spike, where it was validated against a live
// forward and backward seek.
// ---------------------------------------------------------------------------

struct Anchor {
    identity: String,
    reported_position_ms: u64,
    published_position_ms: u64,
    published_updated_at: u64,
}

impl Anchor {
    fn new(identity: &str, position_ms: u64, anchor_ticks: i64) -> Self {
        Self {
            identity: identity.to_owned(),
            reported_position_ms: position_ms,
            published_position_ms: position_ms,
            published_updated_at: anchor_unix_ms(anchor_ticks),
        }
    }

    /// Folds one timeline sample in.
    ///
    /// A sample identical to the one already held is deliberately ignored, so the
    /// published anchor keeps ageing and the frontend keeps counting. Re-anchoring
    /// here is what would pin a stuck source at `0:00` for a whole track.
    fn observe(&mut self, identity: &str, position_ms: u64, anchor_ticks: i64) {
        let unchanged = self.identity == identity && self.reported_position_ms == position_ms;
        if unchanged {
            return;
        }

        self.identity = identity.to_owned();
        self.reported_position_ms = position_ms;
        self.published_position_ms = position_ms;
        self.published_updated_at = anchor_unix_ms(anchor_ticks);
    }
}

/// Converts a WinRT `DateTime` to epoch milliseconds, falling back to now when
/// the source never set one (observed on idle Apple Music sessions).
fn anchor_unix_ms(anchor_ticks: i64) -> u64 {
    if anchor_ticks <= 0 {
        return now_unix_ms();
    }

    let unix_ticks = anchor_ticks - UNIX_EPOCH_IN_1601_TICKS;
    if unix_ticks <= 0 {
        return now_unix_ms();
    }

    u64::try_from(unix_ticks / 10_000).unwrap_or_else(|_| now_unix_ms())
}

/// Drives a WinRT async operation to completion on this thread.
///
/// The worker is a dedicated thread whose whole job is waiting on SMTC, so
/// blocking it is correct and avoids pulling in an async runtime for two calls.
fn block_on<F: IntoFuture>(operation: F) -> F::Output {
    struct ThreadWaker(thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(operation.into_future());

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn ticks_to_ms(ticks: i64) -> u64 {
    u64::try_from(ticks.max(0) / 10_000).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Metadata hygiene
// ---------------------------------------------------------------------------

/// Splits Apple Music's packed `Artist — Album` into its two halves.
///
/// Apple Music leaves `AlbumTitle` empty and repeats the packed string in
/// `AlbumArtist`, so splitting is the only route. Split once, since either half
/// can contain its own dash: `YUNGBLUD — Abyss (from Kaiju No. 8) - Single`.
fn split_artist(raw: &str) -> (Option<String>, Option<String>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (None, None);
    }

    let (artist, album) = match trimmed.split_once(" \u{2014} ") {
        Some((artist, album)) => (artist, Some(album)),
        None => (trimmed, None),
    };

    (
        non_empty(clean_artist(artist)),
        album.and_then(|album| non_empty(clean_album(album))),
    )
}

/// Strips YouTube's auto-generated ` - Topic` channel suffix.
fn clean_artist(artist: &str) -> String {
    artist
        .trim()
        .strip_suffix(" - Topic")
        .unwrap_or(artist.trim())
        .trim()
        .to_owned()
}

/// Strips Apple's ` - Single` and ` - EP` suffixes, which carry no information.
fn clean_album(album: &str) -> String {
    let trimmed = album.trim();
    let stripped = trimmed
        .strip_suffix(" - Single")
        .or_else(|| trimmed.strip_suffix(" - EP"))
        .unwrap_or(trimmed);

    stripped.trim().to_owned()
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

// ---------------------------------------------------------------------------
// Per-session event subscriptions
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Subscriptions {
    held: Vec<Held>,
}

struct Held {
    session: GlobalSystemMediaTransportControlsSession,
    source_app: String,
    media: i64,
    playback: i64,
    timeline: i64,
}

impl Subscriptions {
    /// Re-subscribes only when the set of sessions actually changed, so an
    /// ordinary track change does not churn event registrations.
    fn sync(
        &mut self,
        manager: &GlobalSystemMediaTransportControlsSessionManager,
        tx: &Sender<()>,
    ) -> Result<()> {
        let sessions = manager.GetSessions()?;

        let mut current = Vec::new();
        for index in 0..sessions.Size()? {
            let session = sessions.GetAt(index)?;
            let id = session
                .SourceAppUserModelId()
                .map(|id| id.to_string())
                .unwrap_or_default();
            current.push((id, session));
        }

        let unchanged = current.len() == self.held.len()
            && current
                .iter()
                .zip(self.held.iter())
                .all(|((id, _), held)| *id == held.source_app);

        if unchanged {
            return Ok(());
        }

        self.clear();

        for (source_app, session) in current {
            let media_tx = tx.clone();
            let media = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = media_tx.send(());
                Ok(())
            }))?;

            let playback_tx = tx.clone();
            let playback = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = playback_tx.send(());
                Ok(())
            }))?;

            let timeline_tx = tx.clone();
            let timeline =
                session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = timeline_tx.send(());
                    Ok(())
                }))?;

            self.held.push(Held {
                session,
                source_app,
                media,
                playback,
                timeline,
            });
        }

        Ok(())
    }

    fn clear(&mut self) {
        for held in self.held.drain(..) {
            let _ = held.session.RemoveMediaPropertiesChanged(held.media);
            let _ = held.session.RemovePlaybackInfoChanged(held.playback);
            let _ = held.session.RemoveTimelinePropertiesChanged(held.timeline);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_apple_music_packed_artist() {
        let (artist, album) = split_artist("The Weeknd \u{2014} After Hours (Deluxe)");
        assert_eq!(artist.as_deref(), Some("The Weeknd"));
        assert_eq!(album.as_deref(), Some("After Hours (Deluxe)"));
    }

    #[test]
    fn splits_only_once_so_album_dashes_survive() {
        let (artist, album) = split_artist("YUNGBLUD \u{2014} Abyss (from Kaiju No. 8) - Single");
        assert_eq!(artist.as_deref(), Some("YUNGBLUD"));
        // The ` - Single` suffix is stripped, the inner text is untouched.
        assert_eq!(album.as_deref(), Some("Abyss (from Kaiju No. 8)"));
    }

    #[test]
    fn strips_youtube_topic_channels() {
        let (artist, album) = split_artist("Fei Yu-ching - Topic");
        assert_eq!(artist.as_deref(), Some("Fei Yu-ching"));
        assert_eq!(album, None);
    }

    #[test]
    fn blank_artist_yields_nothing() {
        assert_eq!(split_artist("   "), (None, None));
    }

    #[test]
    fn idle_sessions_never_outrank_playing_ones() {
        assert!(rank_of(SmtcStatus::Playing) > rank_of(SmtcStatus::Paused));
        assert!(rank_of(SmtcStatus::Paused) > rank_of(SmtcStatus::Stopped));
        assert_eq!(rank_of(SmtcStatus::Opened), 0);
        assert_eq!(rank_of(SmtcStatus::Closed), 0);
    }

    /// The Edge case: a source pinned at one position must keep its original
    /// anchor so the frontend's extrapolation keeps advancing.
    #[test]
    fn republished_position_keeps_the_original_anchor() {
        let ticks = UNIX_EPOCH_IN_1601_TICKS + 1_000 * 10_000;
        let mut anchor = Anchor::new("track", 5_000, ticks);
        let first_updated_at = anchor.published_updated_at;

        // Same snapshot arrives again with a much fresher timestamp.
        anchor.observe("track", 5_000, ticks + 30_000 * 10_000);

        assert_eq!(anchor.published_position_ms, 5_000);
        assert_eq!(anchor.published_updated_at, first_updated_at);
    }

    #[test]
    fn a_backward_seek_re_anchors() {
        let ticks = UNIX_EPOCH_IN_1601_TICKS + 1_000 * 10_000;
        let mut anchor = Anchor::new("track", 83_000, ticks);

        anchor.observe("track", 53_000, ticks + 5_000 * 10_000);

        assert_eq!(anchor.published_position_ms, 53_000);
        assert_eq!(anchor.published_updated_at, 6_000);
    }

    #[test]
    fn a_track_change_re_anchors_even_at_the_same_position() {
        let ticks = UNIX_EPOCH_IN_1601_TICKS + 1_000 * 10_000;
        let mut anchor = Anchor::new("first", 0, ticks);

        anchor.observe("second", 0, ticks + 9_000 * 10_000);

        assert_eq!(anchor.published_updated_at, 10_000);
    }

    #[test]
    fn a_missing_anchor_falls_back_to_now() {
        assert!(anchor_unix_ms(0) > 0);
    }
}
