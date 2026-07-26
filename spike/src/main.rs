use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::Result;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

fn main() -> Result<()> {
    // SAFETY: This is the first COM/WinRT initialization on the process's main
    // thread, and every WinRT call in this spike remains on that thread.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
    let once = std::env::args_os().any(|argument| argument == "--once");
    block_on(run(once))
}

async fn run(once: bool) -> Result<()> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.await?;

    println!("vinyl SMTC spike");
    println!("Printing every media session once per second. Press Ctrl+C to stop.\n");

    let mut anchors: HashMap<String, Anchor> = HashMap::new();

    loop {
        let started = Instant::now();

        if let Err(error) = print_snapshot(&manager, &mut anchors).await {
            eprintln!("SMTC snapshot failed: {error}");
        }

        if once {
            return Ok(());
        }

        thread::sleep(Duration::from_secs(1).saturating_sub(started.elapsed()));
    }
}

async fn print_snapshot(
    manager: &GlobalSystemMediaTransportControlsSessionManager,
    anchors: &mut HashMap<String, Anchor>,
) -> Result<()> {
    let sessions = manager.GetSessions()?;
    let current_session = manager.GetCurrentSession().ok();
    let current_summary = match current_session.as_ref() {
        Some(session) => describe_current_session(session)
            .await
            .unwrap_or_else(|error| format!("unreadable ({error})")),
        None => "none".to_owned(),
    };

    println!("--- current: {current_summary} ---");

    if sessions.Size()? == 0 {
        println!("(no media sessions)\n");
        return Ok(());
    }

    for index in 0..sessions.Size()? {
        let session = sessions.GetAt(index)?;
        if let Err(error) = print_session(&session, anchors).await {
            let app_id = session
                .SourceAppUserModelId()
                .map(|id| id.to_string())
                .unwrap_or_else(|_| "unknown app".to_owned());
            eprintln!("[{app_id}] could not read session: {error}");
        }
    }

    println!();
    Ok(())
}

async fn describe_current_session(
    session: &GlobalSystemMediaTransportControlsSession,
) -> Result<String> {
    let app_id = session.SourceAppUserModelId()?.to_string();
    let media = session.TryGetMediaPropertiesAsync()?.await?;
    Ok(format!("{app_id} | \"{}\"", media.Title()?))
}

async fn print_session(
    session: &GlobalSystemMediaTransportControlsSession,
    anchors: &mut HashMap<String, Anchor>,
) -> Result<()> {
    let app_id = session.SourceAppUserModelId()?.to_string();
    let media = session.TryGetMediaPropertiesAsync()?.await?;
    let playback = session.GetPlaybackInfo()?;
    let timeline = session.GetTimelineProperties()?;

    let title = media.Title()?.to_string();
    let artist = media.Artist()?.to_string();
    // Apple Music appears to pack "Artist — Album" into Artist. Print the dedicated
    // album fields so we can tell whether clean values are available separately.
    let album_title = media
        .AlbumTitle()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let album_artist = media
        .AlbumArtist()
        .map(|v| v.to_string())
        .unwrap_or_default();
    let playback_status = playback.PlaybackStatus()?;
    let status = status_name(playback_status);
    let position_ticks = timeline.Position()?.Duration;
    let duration_ticks = timeline.EndTime()?.Duration;
    let position = format_timespan(position_ticks);
    let duration = format_timespan(duration_ticks);
    let thumbnail_bytes = read_thumbnail_bytes(&media).await?;

    // The anchor age is the whole ballgame for CLAUDE.md constraint 1: a source that
    // freezes Position but lets its timestamp go stale still extrapolates correctly,
    // while one that refreshes the timestamp without moving Position cannot.
    let anchor_ticks = timeline.LastUpdatedTime()?.UniversalTime;
    let anchor = describe_anchor(
        anchor_ticks,
        position_ticks,
        duration_ticks,
        playback_status,
    );

    let playing =
        playback_status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;
    let reckoned_ms = reconcile(
        anchors,
        &app_id,
        format!("{title}|{artist}"),
        position_ticks,
        anchor_ticks,
        duration_ticks,
        playing,
    );
    let reckoned = format_timespan(reckoned_ms.saturating_mul(10_000));

    println!(
        "[{app_id}] {status} | title={title:?} artist={artist:?} album={album_title:?} albumArtist={album_artist:?} | {position}/{duration} | {anchor} | reckoned {reckoned} | art: {} | {} bytes",
        if thumbnail_bytes.is_some() {
            "yes"
        } else {
            "no"
        },
        thumbnail_bytes.as_ref().map_or(0, Vec::len),
    );

    Ok(())
}

/// A locally maintained playback clock, the thing the real widget would count from.
///
/// The source's own `Position` cannot be trusted to move, and for some sources its
/// `LastUpdatedTime` refreshes without the position advancing, which defeats plain
/// extrapolation. So we hold our own base and only reset it when the source tells us
/// something genuinely new.
struct Anchor {
    identity: String,
    reported_position: i64,
    base_ms: i64,
    base_instant: Instant,
    playing: bool,
}

impl Anchor {
    fn effective_ms(&self) -> i64 {
        if self.playing {
            let elapsed =
                i64::try_from(self.base_instant.elapsed().as_millis()).unwrap_or(i64::MAX);
            self.base_ms.saturating_add(elapsed)
        } else {
            self.base_ms
        }
    }
}

/// Folds one SMTC timeline sample into the local clock and returns what to display.
///
/// The rule that matters: when the source republishes a position identical to the one
/// we already hold, we deliberately do **not** re-anchor. Re-anchoring there is what
/// would pin a stuck source at `0:00` forever.
fn reconcile(
    anchors: &mut HashMap<String, Anchor>,
    app_id: &str,
    identity: String,
    position_ticks: i64,
    anchor_ticks: i64,
    duration_ticks: i64,
    playing: bool,
) -> i64 {
    let republished = anchors
        .get(app_id)
        .is_some_and(|held| held.identity == identity && held.reported_position == position_ticks);

    if republished {
        // Nothing new from the source. Keep counting, but honour a play/pause flip so
        // the clock freezes and resumes from where it actually stood.
        if let Some(held) = anchors.get_mut(app_id) {
            if held.playing != playing {
                held.base_ms = held.effective_ms();
                held.base_instant = Instant::now();
                held.playing = playing;
            }
        }
    } else {
        // New track, a genuine push, or a seek. Trust the source, and credit however
        // long its snapshot has already been sitting stale.
        let position_ms = position_ticks.max(0) / 10_000;
        let remote_age_ms = if anchor_ticks > 0 {
            ticks_since_1601().saturating_sub(anchor_ticks).max(0) / 10_000
        } else {
            0
        };

        anchors.insert(
            app_id.to_owned(),
            Anchor {
                identity,
                reported_position: position_ticks,
                base_ms: if playing {
                    position_ms.saturating_add(remote_age_ms)
                } else {
                    position_ms
                },
                base_instant: Instant::now(),
                playing,
            },
        );
    }

    let effective = anchors.get(app_id).map_or(0, Anchor::effective_ms);
    let duration_ms = duration_ticks.max(0) / 10_000;

    if duration_ms > 0 {
        effective.min(duration_ms)
    } else {
        effective
    }
}

/// Reports how stale the timeline snapshot is and what the extrapolation in
/// `CLAUDE.md` constraint 1 would render from it.
fn describe_anchor(
    anchor_ticks: i64,
    position_ticks: i64,
    duration_ticks: i64,
    status: GlobalSystemMediaTransportControlsSessionPlaybackStatus,
) -> String {
    if anchor_ticks <= 0 {
        return "anchor: never set".to_owned();
    }

    let age_ticks = ticks_since_1601().saturating_sub(anchor_ticks);
    if age_ticks < 0 {
        return "anchor: in the future".to_owned();
    }

    let age_seconds = age_ticks as f64 / 10_000_000.0;

    if status != GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing {
        return format!("anchor {age_seconds:.1}s ago");
    }

    let mut extrapolated = position_ticks.saturating_add(age_ticks);
    if duration_ticks > 0 {
        extrapolated = extrapolated.min(duration_ticks);
    }

    format!(
        "anchor {age_seconds:.1}s ago, extrapolated {}",
        format_timespan(extrapolated)
    )
}

/// Current time as 100 ns ticks since 1601-01-01, the epoch WinRT `DateTime` uses.
fn ticks_since_1601() -> i64 {
    const UNIX_EPOCH_IN_1601_TICKS: i64 = 116_444_736_000_000_000;

    let since_unix_epoch = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => (elapsed.as_nanos() / 100) as i64,
        Err(_) => 0,
    };

    UNIX_EPOCH_IN_1601_TICKS.saturating_add(since_unix_epoch)
}

async fn read_thumbnail_bytes(
    media: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Result<Option<Vec<u8>>> {
    let reference = match media.Thumbnail() {
        Ok(reference) => reference,
        Err(_) => return Ok(None),
    };

    let stream = reference.OpenReadAsync()?.await?;
    let size = stream.Size()?;
    if size == 0 {
        return Ok(None);
    }

    let readable_size = u32::try_from(size).unwrap_or(u32::MAX);
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(readable_size)?.await?;
    let mut bytes = vec![0; loaded as usize];
    reader.ReadBytes(&mut bytes)?;

    Ok(Some(bytes))
}

fn block_on<F: Future>(future: F) -> F::Output {
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
    let mut future = std::pin::pin!(future);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

fn format_timespan(ticks_100ns: i64) -> String {
    if ticks_100ns <= 0 {
        return "0:00".to_owned();
    }

    let total_seconds = ticks_100ns as u64 / 10_000_000;
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn status_name(status: GlobalSystemMediaTransportControlsSessionPlaybackStatus) -> &'static str {
    match status {
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed => "closed",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Opened => "opened",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Changing => "changing",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped => "stopped",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing => "playing",
        GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused => "paused",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{format_timespan, reconcile, Anchor};
    use std::collections::HashMap;
    use std::thread;
    use std::time::Duration;

    const APP: &str = "test.exe";
    const DURATION_TICKS: i64 = 416 * 10_000_000;

    fn sample(anchors: &mut HashMap<String, Anchor>, position_ticks: i64, playing: bool) -> i64 {
        reconcile(
            anchors,
            APP,
            "title|artist".to_owned(),
            position_ticks,
            0,
            DURATION_TICKS,
            playing,
        )
    }

    /// The Apple Music failure mode: a source pinned at 0:00 that never moves. The
    /// local clock must advance anyway, otherwise the tonearm never leaves the edge.
    #[test]
    fn republished_position_still_advances() {
        let mut anchors = HashMap::new();

        assert_eq!(sample(&mut anchors, 0, true), 0);
        thread::sleep(Duration::from_millis(120));

        let later = sample(&mut anchors, 0, true);
        assert!(
            later >= 100,
            "expected the clock to advance, got {later} ms"
        );
    }

    /// A seek must win over the local clock rather than being smoothed away.
    #[test]
    fn changed_position_re_anchors() {
        let mut anchors = HashMap::new();

        sample(&mut anchors, 0, true);
        thread::sleep(Duration::from_millis(120));

        let seeked = sample(&mut anchors, 60 * 10_000_000, true);
        assert!(
            (60_000..60_500).contains(&seeked),
            "seek should re-anchor near 60 s, got {seeked} ms"
        );
    }

    /// A backward scrub must pull the clock back, not keep counting up. Confirmed
    /// against Apple Music seeking from 1:23 to 0:53.
    #[test]
    fn backward_seek_re_anchors() {
        let mut anchors = HashMap::new();

        sample(&mut anchors, 83 * 10_000_000, true);
        thread::sleep(Duration::from_millis(120));

        let seeked = sample(&mut anchors, 53 * 10_000_000, true);
        assert!(
            (53_000..53_500).contains(&seeked),
            "backward seek should re-anchor near 53 s, got {seeked} ms"
        );
    }

    /// A paused session must not creep forward.
    #[test]
    fn paused_clock_holds() {
        let mut anchors = HashMap::new();

        let first = sample(&mut anchors, 30 * 10_000_000, false);
        thread::sleep(Duration::from_millis(120));

        assert_eq!(first, 30_000);
        assert_eq!(sample(&mut anchors, 30 * 10_000_000, false), 30_000);
    }

    /// Pausing freezes wherever the clock actually stood, not at the stale snapshot.
    #[test]
    fn pause_captures_accumulated_progress() {
        let mut anchors = HashMap::new();

        sample(&mut anchors, 0, true);
        thread::sleep(Duration::from_millis(120));

        let frozen = sample(&mut anchors, 0, false);
        assert!(frozen >= 100, "pause should keep progress, got {frozen} ms");
        thread::sleep(Duration::from_millis(60));
        assert_eq!(sample(&mut anchors, 0, false), frozen);
    }

    #[test]
    fn formats_zero_and_negative_as_zero() {
        assert_eq!(format_timespan(0), "0:00");
        assert_eq!(format_timespan(-1), "0:00");
    }

    #[test]
    fn formats_minutes_and_hours() {
        assert_eq!(format_timespan(62 * 10_000_000), "1:02");
        assert_eq!(format_timespan(3_662 * 10_000_000), "1:01:02");
    }
}
