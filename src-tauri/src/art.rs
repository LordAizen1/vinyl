//! In-memory cache for album art, served to the frontend over the `art://`
//! scheme so the bytes never cross the Tauri bridge as base64.
//!
//! Two constraints shape this. `ASSETS.md` requires album art be cached in
//! memory only and dropped on exit: it belongs to whoever owns the recording,
//! and we are only allowed to display it transiently. And Phase 0 measured a
//! single Apple Music thumbnail at 1,022,489 bytes, so the cache is small and
//! bounded rather than keeping everything it has ever seen.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::RwLock;

/// Enough for the current track plus recent history, so flicking back and
/// forth does not re-decode. Eight megabyte-ish entries is the worst case.
const MAX_ENTRIES: usize = 8;

#[derive(Default)]
pub struct ArtCache {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    bytes: HashMap<String, Arc<Vec<u8>>>,
    order: VecDeque<String>,
}

impl ArtCache {
    /// Stores the bytes and returns their `art_id`.
    ///
    /// Identical bytes always produce the same id, so the frontend only
    /// refetches when the artwork has genuinely changed.
    pub fn insert(&self, bytes: Vec<u8>) -> String {
        let id = fingerprint(&bytes);
        let mut inner = self.inner.write();

        if inner.bytes.contains_key(&id) {
            return id;
        }

        inner.bytes.insert(id.clone(), Arc::new(bytes));
        inner.order.push_back(id.clone());

        while inner.order.len() > MAX_ENTRIES {
            if let Some(evicted) = inner.order.pop_front() {
                inner.bytes.remove(&evicted);
            }
        }

        id
    }

    pub fn get(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        self.inner.read().bytes.get(id).cloned()
    }
}

/// FNV-1a over the raw bytes. Not cryptographic; the only question it answers
/// is "are these the same bytes as last time".
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Sniffs the image type from its magic bytes.
///
/// SMTC gives us no content type, and sources vary: browsers tend to hand over
/// PNG, Apple Music JPEG.
pub fn content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_bytes_share_an_id() {
        let cache = ArtCache::default();
        let first = cache.insert(vec![1, 2, 3]);
        let second = cache.insert(vec![1, 2, 3]);
        assert_eq!(first, second);
    }

    #[test]
    fn different_bytes_get_different_ids() {
        let cache = ArtCache::default();
        assert_ne!(cache.insert(vec![1, 2, 3]), cache.insert(vec![3, 2, 1]));
    }

    #[test]
    fn stored_bytes_come_back() {
        let cache = ArtCache::default();
        let id = cache.insert(vec![9, 9, 9]);
        assert_eq!(cache.get(&id).as_deref(), Some(&vec![9, 9, 9]));
    }

    #[test]
    fn the_cache_stays_bounded() {
        let cache = ArtCache::default();
        let first = cache.insert(vec![0]);
        for n in 1..=MAX_ENTRIES {
            cache.insert(vec![u8::try_from(n).unwrap()]);
        }
        assert!(
            cache.get(&first).is_none(),
            "oldest entry should be evicted"
        );
    }

    #[test]
    fn unknown_ids_miss() {
        assert!(ArtCache::default().get("nope").is_none());
    }

    #[test]
    fn sniffs_the_common_formats() {
        assert_eq!(content_type(&[0x89, b'P', b'N', b'G', 0]), "image/png");
        assert_eq!(content_type(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(content_type(b"nonsense"), "application/octet-stream");
    }
}
