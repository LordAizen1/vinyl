//! The single state type the whole app is built around.
//!
//! Everything upstream of this module exists to fill one of these in; everything
//! downstream is presentation. See `CLAUDE.md` for the rationale.

use std::sync::Arc;

use parking_lot::RwLock;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Playing,
    Paused,
    Stopped,
    NoSession,
}

/// A snapshot of what is playing anywhere on the system.
///
/// `position_ms` is deliberately **not** live. It is the position as of
/// `updated_at`, and the frontend extrapolates from there. See `CLAUDE.md`
/// constraint 1; SMTC sources routinely leave a position untouched for minutes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// Hash of the art bytes. Always `None` until Phase 3 wires up artwork.
    pub art_id: Option<String>,
    pub status: Status,
    pub position_ms: Option<u64>,
    /// `None` or zero means unknown, which is what livestreams report.
    pub duration_ms: Option<u64>,
    /// Epoch milliseconds at which `position_ms` was true.
    pub updated_at: u64,
    /// The source AUMID, for example `Spotify.exe` or `msedge.exe`.
    pub source_app: String,
    /// Audio peak in 0.0..=1.0. Always zero until Phase 6.
    pub peak: f32,
}

impl PlaybackState {
    /// The state shown when nothing at all is playing.
    pub fn no_session() -> Self {
        Self {
            title: None,
            artist: None,
            album: None,
            art_id: None,
            status: Status::NoSession,
            position_ms: None,
            duration_ms: None,
            updated_at: 0,
            source_app: String::new(),
            peak: 0.0,
        }
    }

    /// Whether two snapshots differ in any way worth waking the frontend for.
    ///
    /// Compared field by field rather than by timestamp, so a source that
    /// republishes an unchanged snapshot does not cause a pointless emit.
    pub fn differs_from(&self, other: &Self) -> bool {
        self.title != other.title
            || self.artist != other.artist
            || self.album != other.album
            || self.art_id != other.art_id
            || self.status != other.status
            || self.position_ms != other.position_ms
            || self.duration_ms != other.duration_ms
            || self.updated_at != other.updated_at
            || self.source_app != other.source_app
    }
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self::no_session()
    }
}

/// The one shared instance, read by the Tauri command layer and written by the
/// SMTC worker thread.
pub type SharedState = Arc<RwLock<PlaybackState>>;

pub fn shared() -> SharedState {
    Arc::new(RwLock::new(PlaybackState::no_session()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_session_is_the_default() {
        assert_eq!(PlaybackState::default().status, Status::NoSession);
        assert!(PlaybackState::default().title.is_none());
    }

    #[test]
    fn identical_snapshots_do_not_differ() {
        let state = PlaybackState::no_session();
        assert!(!state.differs_from(&state.clone()));
    }

    #[test]
    fn a_changed_position_counts_as_different() {
        let first = PlaybackState::no_session();
        let mut second = first.clone();
        second.position_ms = Some(1_000);
        assert!(first.differs_from(&second));
    }
}
