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
use std::sync::mpsc::{self, Receiver, Sender};
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
use windows::Media::MediaPlaybackAutoRepeatMode as SmtcRepeat;
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::art::ArtCache;
use crate::state::{PlaybackState, RepeatMode, SharedState, Status};

/// Safety net only. Events drive the updates; this bounds how long a missed
/// event can leave the UI stale. Reading metadata is cheap because Phase 1 does
/// not touch thumbnails, which are the expensive part (measured at 1 MB).
const WATCHDOG: Duration = Duration::from_secs(5);

/// How long to wait between attempts while the artwork still looks stale.
/// Only ever used for the first second or so after a track change.
const SETTLE_POLL: Duration = Duration::from_millis(280);

/// Unix epoch expressed in the 100 ns ticks since 1601 that WinRT `DateTime` uses.
const UNIX_EPOCH_IN_1601_TICKS: i64 = 116_444_736_000_000_000;

/// A transport action, sent from the Tauri command layer.
#[derive(Debug, Clone, Copy)]
pub enum Command {
    Toggle,
    Next,
    Previous,
    /// Flip shuffle. The worker reads the session's current value rather than
    /// trusting one sent from the UI, which may be a frame out of date.
    ToggleShuffle,
    /// Advance repeat: off, then whole list, then single track, then off.
    CycleRepeat,
}

/// Why the worker woke up.
///
/// Transport commands travel on the same channel as change notifications so
/// they run on the worker's own MTA thread. Calling into a session from the UI
/// thread would mean initialising COM there too, for no benefit.
pub enum Signal {
    Poll,
    Run(Command),
}

/// Starts the worker and returns the handle used to send it transport commands.
pub fn spawn(app: AppHandle, state: SharedState, art: Arc<ArtCache>) -> Sender<Signal> {
    let (tx, rx) = mpsc::channel::<Signal>();
    let worker_tx = tx.clone();

    thread::spawn(move || {
        if let Err(error) = run(&app, &state, &art, &worker_tx, &rx) {
            log::error!("SMTC worker stopped: {error:#}");
        }
    });

    tx
}

fn run(
    app: &AppHandle,
    state: &SharedState,
    art: &ArtCache,
    tx: &Sender<Signal>,
    rx: &Receiver<Signal>,
) -> Result<()> {
    // SAFETY: first WinRT call on this thread, and every WinRT call made by this
    // worker stays on it. MTA means event callbacks arrive on pool threads
    // without needing a message pump.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };

    let manager = block_on(GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?)?;

    let sessions_tx = tx.clone();
    manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = sessions_tx.send(Signal::Poll);
        Ok(())
    }))?;

    let current_tx = tx.clone();
    manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = current_tx.send(Signal::Poll);
        Ok(())
    }))?;

    let mut subscriptions = Subscriptions::default();
    let mut anchors: HashMap<String, Anchor> = HashMap::new();
    let mut artwork = ArtTracker::default();
    let mut selected: Option<GlobalSystemMediaTransportControlsSession> = None;

    loop {
        if let Err(error) = subscriptions.sync(&manager, tx) {
            log::warn!("could not refresh session subscriptions: {error:#}");
        }

        match read(&manager, &mut anchors, &mut artwork, art, &mut selected) {
            Ok(snapshot) => publish(app, state, snapshot),
            Err(error) => log::warn!("SMTC read failed: {error:#}"),
        }

        // Wait for an event, or fall through on the watchdog. While the
        // artwork is still settling after a track change, look again sooner:
        // sources publish the new title before the new image.
        let wait = if artwork.is_settling() {
            SETTLE_POLL
        } else {
            WATCHDOG
        };

        // Coalesce a burst; sources fire several change events per track
        // change. Commands are never coalesced away.
        let mut wakes = Vec::new();
        if let Ok(wake) = rx.recv_timeout(wait) {
            wakes.push(wake);
        }
        while let Ok(wake) = rx.try_recv() {
            wakes.push(wake);
        }

        for wake in wakes {
            if let Signal::Run(command) = wake {
                match &selected {
                    Some(session) => execute(session, command),
                    None => log::warn!("transport: {command:?} with no session selected"),
                }
            }
        }
    }
}

/// Sends a transport command to the selected session.
///
/// A `false` return means the source accepted the call and declined to act,
/// which is different from an error and worth seeing separately in the log.
fn execute(session: &GlobalSystemMediaTransportControlsSession, command: Command) {
    let outcome = match command {
        Command::Toggle => session.TryTogglePlayPauseAsync().and_then(block_on),
        Command::Next => session.TrySkipNextAsync().and_then(block_on),
        Command::Previous => session.TrySkipPreviousAsync().and_then(block_on),
        Command::ToggleShuffle => session
            .TryChangeShuffleActiveAsync(!read_shuffle(session))
            .and_then(block_on),
        Command::CycleRepeat => session
            .TryChangeAutoRepeatModeAsync(next_repeat(read_repeat(session)))
            .and_then(block_on),
    };

    match outcome {
        Ok(true) => log::info!("transport: {command:?} accepted"),
        Ok(false) => log::warn!("transport: {command:?} refused by the source"),
        Err(error) => log::warn!("transport: {command:?} failed ({error})"),
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
    selected: &mut Option<GlobalSystemMediaTransportControlsSession>,
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
        *selected = None;
        // Settled, not pending: with no session there is nothing to look for,
        // and leaving it unsettled would spin the worker at SETTLE_POLL.
        *artwork = ArtTracker::idle();
        return Ok(PlaybackState::no_session());
    };

    // Held so transport commands act on whatever the widget is showing, which
    // is not necessarily what GetCurrentSession would have picked.
    *selected = Some(session.clone());

    describe(&session, anchors, artwork, art)
}

/// Reads shuffle from a session.
///
/// SMTC returns these as nullable references, and plenty of sources simply do
/// not set them, so absent means off rather than being an error.
fn read_shuffle(session: &GlobalSystemMediaTransportControlsSession) -> bool {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.IsShuffleActive())
        .and_then(|value| value.Value())
        .unwrap_or(false)
}

fn read_repeat(session: &GlobalSystemMediaTransportControlsSession) -> SmtcRepeat {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.AutoRepeatMode())
        .and_then(|value| value.Value())
        .unwrap_or(SmtcRepeat::None)
}

/// Off, then the whole list, then the single track, then off again. Matches
/// the order every media player uses, so the button behaves as expected.
fn next_repeat(current: SmtcRepeat) -> SmtcRepeat {
    match current {
        SmtcRepeat::None => SmtcRepeat::List,
        SmtcRepeat::List => SmtcRepeat::Track,
        _ => SmtcRepeat::None,
    }
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

    // Reading the thumbnail is the expensive part of this module: Phase 0
    // measured one at 1,022,489 bytes, and doing that per tick would not fit
    // the idle CPU budget in CLAUDE.md constraint 4. So it runs on a track
    // change, and then again only while the result still looks like the
    // previous track's art.
    let art_key = format!("{source_app}|{identity}");
    if artwork.identity != art_key {
        log::info!("art: track changed to {art_key:?}");
        artwork.begin_track(art_key);
    }

    if artwork.is_settling() {
        artwork.attempts += 1;
        if let Some(bytes) = read_thumbnail(&media) {
            let id = art.insert(bytes);
            if artwork.art_id.as_ref() != Some(&id) {
                log::info!("art: art_id is now {id} (attempt {})", artwork.attempts);
            }
            artwork.art_id = Some(id);
        }
    }

    // Only offer a control the source actually supports. A dead skip button on
    // a livestream is a bug, not a cosmetic detail.
    let controls = playback.Controls().ok();
    let can_play_pause = controls
        .as_ref()
        .map(|c| {
            c.IsPlayEnabled().unwrap_or(false)
                || c.IsPauseEnabled().unwrap_or(false)
                || c.IsPlayPauseToggleEnabled().unwrap_or(false)
        })
        .unwrap_or(false);
    let can_next = controls
        .as_ref()
        .and_then(|c| c.IsNextEnabled().ok())
        .unwrap_or(false);
    let can_previous = controls
        .as_ref()
        .and_then(|c| c.IsPreviousEnabled().ok())
        .unwrap_or(false);
    let can_shuffle = controls
        .as_ref()
        .and_then(|c| c.IsShuffleEnabled().ok())
        .unwrap_or(false);
    let can_repeat = controls
        .as_ref()
        .and_then(|c| c.IsRepeatEnabled().ok())
        .unwrap_or(false);

    let repeat = match read_repeat(session) {
        SmtcRepeat::Track => RepeatMode::Track,
        SmtcRepeat::List => RepeatMode::List,
        _ => RepeatMode::Off,
    };

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
        can_play_pause,
        can_next,
        can_previous,
        can_shuffle,
        can_repeat,
        shuffle: read_shuffle(session),
        repeat,
    })
}

// ---------------------------------------------------------------------------
// Artwork
// ---------------------------------------------------------------------------

/// How many times to re-read a thumbnail before accepting what we have.
///
/// Sources publish the new title before the new artwork, so the read triggered
/// by a track change frequently returns the *previous* track's image. Each
/// attempt can cost a megabyte, so this is deliberately small and only runs
/// while the art still looks stale.
const MAX_ART_ATTEMPTS: u8 = 4;

/// Remembers which track's art we hold, so the thumbnail is read once per
/// track rather than once per event, and so a stale read can be corrected.
#[derive(Default)]
struct ArtTracker {
    identity: String,
    art_id: Option<String>,
    /// What the previous track's art hashed to. If a fresh read returns this,
    /// the source has not caught up yet and we should look again.
    previous_art_id: Option<String>,
    attempts: u8,
}

impl ArtTracker {
    /// Whether the art we hold is still suspect.
    ///
    /// True when we have nothing, or when what we have is byte-identical to
    /// the previous track's art. The latter is occasionally legitimate, two
    /// tracks off one album, which is why attempts are capped rather than
    /// looped until they differ.
    fn is_settling(&self) -> bool {
        self.attempts < MAX_ART_ATTEMPTS
            && (self.art_id.is_none() || self.art_id == self.previous_art_id)
    }

    fn begin_track(&mut self, identity: String) {
        self.previous_art_id = self.art_id.take();
        self.identity = identity;
        self.attempts = 0;
    }

    /// A tracker with nothing to look for, so the worker goes back to sleep.
    fn idle() -> Self {
        Self {
            attempts: MAX_ART_ATTEMPTS,
            ..Self::default()
        }
    }
}

/// Pulls the thumbnail bytes out of a session.
///
/// Every step is fallible, and a failure usually just means "no art", which is
/// the common case: Phase 0 found browsers, local files and livestreams
/// routinely have no thumbnail. That is what the procedural label is for. Each
/// step logs its own failure, because "no art" and "art we failed to read" look
/// identical from the outside and need telling apart.
fn read_thumbnail(media: &MediaProperties) -> Option<Vec<u8>> {
    let reference = match media.Thumbnail() {
        Ok(reference) => reference,
        Err(error) => {
            log::info!("art: no thumbnail published ({error})");
            return None;
        }
    };

    let open = match reference.OpenReadAsync() {
        Ok(operation) => operation,
        Err(error) => {
            log::warn!("art: OpenReadAsync call failed ({error})");
            return None;
        }
    };

    let stream = match block_on(open) {
        Ok(stream) => stream,
        Err(error) => {
            log::warn!("art: OpenReadAsync did not complete ({error})");
            return None;
        }
    };

    let size = match stream.Size() {
        Ok(size) => size,
        Err(error) => {
            log::warn!("art: stream has no size ({error})");
            return None;
        }
    };

    if size == 0 {
        log::info!("art: thumbnail stream is empty");
        return None;
    }

    let readable = match u32::try_from(size) {
        Ok(readable) => readable,
        Err(_) => {
            log::warn!("art: thumbnail is implausibly large ({size} bytes)");
            return None;
        }
    };

    let reader = match DataReader::CreateDataReader(&stream) {
        Ok(reader) => reader,
        Err(error) => {
            log::warn!("art: could not create a DataReader ({error})");
            return None;
        }
    };

    let loaded = match reader.LoadAsync(readable).and_then(block_on) {
        Ok(loaded) => loaded,
        Err(error) => {
            log::warn!("art: LoadAsync failed ({error})");
            return None;
        }
    };

    let mut bytes = vec![0_u8; loaded as usize];
    if let Err(error) = reader.ReadBytes(&mut bytes) {
        log::warn!("art: ReadBytes failed after loading {loaded} bytes ({error})");
        return None;
    }

    if bytes.is_empty() {
        log::info!("art: thumbnail read returned zero bytes");
        return None;
    }

    log::info!("art: read {} bytes", bytes.len());
    Some(bytes)
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
        tx: &Sender<Signal>,
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
                let _ = media_tx.send(Signal::Poll);
                Ok(())
            }))?;

            let playback_tx = tx.clone();
            let playback = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = playback_tx.send(Signal::Poll);
                Ok(())
            }))?;

            let timeline_tx = tx.clone();
            let timeline =
                session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    let _ = timeline_tx.send(Signal::Poll);
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

    /// The reported bug: a track change where the source hands back the
    /// previous track's artwork must not be accepted as final.
    #[test]
    fn stale_art_keeps_looking() {
        let mut art = ArtTracker::default();
        art.begin_track("first".to_owned());
        art.attempts = 1;
        art.art_id = Some("aaa".to_owned());
        assert!(!art.is_settling(), "fresh art on a new track is settled");

        art.begin_track("second".to_owned());
        art.attempts = 1;
        // The source has not caught up: same bytes as the previous track.
        art.art_id = Some("aaa".to_owned());
        assert!(
            art.is_settling(),
            "art identical to the last track is suspect"
        );

        // The real image finally arrives.
        art.attempts = 2;
        art.art_id = Some("bbb".to_owned());
        assert!(!art.is_settling());
    }

    #[test]
    fn art_attempts_are_capped() {
        let mut art = ArtTracker::default();
        art.begin_track("only".to_owned());
        art.attempts = MAX_ART_ATTEMPTS;
        assert!(
            !art.is_settling(),
            "must stop re-reading; each attempt can cost a megabyte"
        );
    }

    #[test]
    fn a_session_with_no_art_stops_retrying() {
        let mut art = ArtTracker::default();
        art.begin_track("silent".to_owned());
        for _ in 0..MAX_ART_ATTEMPTS {
            assert!(art.is_settling());
            art.attempts += 1;
        }
        assert!(!art.is_settling());
    }

    #[test]
    fn an_idle_tracker_never_polls() {
        assert!(!ArtTracker::idle().is_settling());
    }
}
