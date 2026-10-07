//! Holds generated media between the tool that made it and the code that
//! attaches it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;

use crate::types::{MediaKind, MimeType};

/// Media to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMedia {
    pub kind: MediaKind,
    pub data: Vec<u8>,
    pub mime_type: MimeType,
    pub prompt: String,
}

/// Held media.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMedia {
    pub id: String,
    pub kind: MediaKind,
    pub data: Vec<u8>,
    pub mime_type: MimeType,
    pub prompt: String,
    pub created_at: i64,
    pub path: Option<PathBuf>,
}

const DEFAULT_TTL: Duration = Duration::from_secs(60 * 60);
const DEFAULT_MAX_ITEMS: usize = 50;

/// Generated media ids look like `img_…` or `vid_…`; anything else was made
/// up. The same shape as the legacy `/^(img|vid)_[a-z0-9]+_\d+$/`.
pub fn is_media_id(id: &str) -> bool {
    parse_id(id).is_some()
}

/// The kind and the creation time (unix ms) an id carries.
fn parse_id(id: &str) -> Option<(MediaKind, i64)> {
    let mut parts = id.split('_');
    let kind = match parts.next()? {
        "img" => MediaKind::Image,
        "vid" => MediaKind::Video,
        _ => return None,
    };
    let stamp = parts.next()?;
    let counter = parts.next()?;
    let well_formed = !stamp.is_empty()
        && stamp.bytes().all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
        && !counter.is_empty()
        && counter.bytes().all(|byte| byte.is_ascii_digit())
        && parts.next().is_none();
    if !well_formed {
        return None;
    }
    Some((kind, i64::from_str_radix(stamp, 36).ok()?))
}

/// `Number#toString(36)` for a non-negative integer.
fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Default)]
struct Held {
    /// Insertion order is age order, so the front is the oldest.
    items: VecDeque<StoredMedia>,
    counter: u64,
}

/// Holds generated media between the tool that made it and the code that
/// attaches it to a post or a chat message.
///
/// In-process on purpose. A generated image lives for one turn: the drafter
/// asks for it, names it in its answer, and the job attaches it minutes later
/// at most. Anything older than the TTL is somebody's abandoned draft, and the
/// cap keeps a chatty day from holding a few hundred megabytes of PNGs. A copy
/// is written to `<output dir>/<id>.<ext>` when a directory is given, the way
/// the legacy store wrote it, and an id this process never held is looked up
/// there, so media made just before a restart can still be attached.
pub struct MediaStore {
    held: Mutex<Held>,
    output_dir: Option<PathBuf>,
    ttl: Duration,
    max_items: usize,
    clock: Clock,
}

impl Default for MediaStore {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MediaStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaStore")
            .field("output_dir", &self.output_dir)
            .field("ttl", &self.ttl)
            .field("max_items", &self.max_items)
            .finish_non_exhaustive()
    }
}

impl MediaStore {
    /// An in-memory store: an hour's TTL, fifty items, no copies on disk.
    pub fn new() -> Self {
        Self {
            held: Mutex::new(Held::default()),
            output_dir: None,
            ttl: DEFAULT_TTL,
            max_items: DEFAULT_MAX_ITEMS,
            clock: Arc::new(|| chrono::Utc::now().timestamp_millis()),
        }
    }

    /// Where copies are written (`MEDIA_OUTPUT_DIR`, `data/media`).
    pub fn with_output_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.output_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// How long an item is held.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// How many items are held at most.
    pub fn with_max_items(mut self, max_items: usize) -> Self {
        self.max_items = max_items;
        self
    }

    /// The time, in unix milliseconds.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        // A panic while holding the lock leaves plain data behind; keep going.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds the media and hands back its id (and the copy's path).
    pub fn put(&self, media: NewMedia) -> StoredMedia {
        let now = self.now();
        let mut held = self.held();
        self.sweep(&mut held, now);
        held.counter += 1;
        let prefix = match media.kind {
            MediaKind::Image => "img",
            MediaKind::Video => "vid",
        };
        let id = format!("{prefix}_{}_{}", base36(now.max(0) as u64), held.counter);
        let mut stored = StoredMedia {
            id,
            kind: media.kind,
            data: media.data,
            mime_type: media.mime_type,
            prompt: media.prompt,
            created_at: now,
            path: None,
        };
        stored.path = self.persist(&stored);
        held.items.push_back(stored.clone());
        stored
    }

    /// The media under this id, from memory or from its copy on disk.
    pub fn get(&self, id: &str) -> Option<StoredMedia> {
        let now = self.now();
        {
            let mut held = self.held();
            self.sweep(&mut held, now);
            if let Some(found) = held.items.iter().find(|item| item.id == id) {
                return Some(found.clone());
            }
        }
        self.from_disk(id, now)
    }

    /// Every item still held in memory, oldest first.
    pub fn list(&self) -> Vec<StoredMedia> {
        let now = self.now();
        let mut held = self.held();
        self.sweep(&mut held, now);
        held.items.iter().cloned().collect()
    }

    fn cutoff(&self, now: i64) -> i64 {
        now - i64::try_from(self.ttl.as_millis()).unwrap_or(i64::MAX)
    }

    /// Drops what is past the TTL, then makes room for one more -- on every
    /// call, reads included, as the legacy store did.
    fn sweep(&self, held: &mut Held, now: i64) {
        let cutoff = self.cutoff(now);
        held.items.retain(|item| item.created_at >= cutoff);
        while !held.items.is_empty() && held.items.len() >= self.max_items {
            held.items.pop_front();
        }
    }

    fn persist(&self, media: &StoredMedia) -> Option<PathBuf> {
        let dir = self.output_dir.as_ref()?;
        let path = dir.join(format!("{}.{}", media.id, media.mime_type.extension()));
        // The copy is a convenience. Failing the generation over a full disk
        // would turn a missing file into a missing reply.
        let written = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, &media.data));
        match written {
            Ok(()) => Some(path),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "could not keep a copy of generated media");
                None
            }
        }
    }

    /// The copy a previous process wrote, while it is inside the TTL. The id
    /// is checked against the id shape first, so it cannot name a path.
    fn from_disk(&self, id: &str, now: i64) -> Option<StoredMedia> {
        let dir = self.output_dir.as_ref()?;
        let (kind, created_at) = parse_id(id)?;
        if created_at < self.cutoff(now) {
            return None;
        }
        let candidates: &[MimeType] = match kind {
            MediaKind::Image => &[MimeType::Png, MimeType::Jpeg, MimeType::Webp],
            MediaKind::Video => &[MimeType::Mp4],
        };
        candidates.iter().find_map(|mime_type| {
            let path = dir.join(format!("{id}.{}", mime_type.extension()));
            let data = std::fs::read(&path).ok()?;
            Some(StoredMedia {
                id: id.to_owned(),
                kind,
                data,
                mime_type: *mime_type,
                prompt: String::new(),
                created_at,
                path: Some(path),
            })
        })
    }
}

/// How much the account may generate per day, counted per kind because a
/// video costs a hundred times what an image does.
///
/// The count lives with the cadence rules, not here; generation takes one of
/// these and asks it before spending anything.
#[async_trait]
pub trait MediaBudget: Send + Sync {
    /// Takes one from today's allowance, or returns false when there is none
    /// left.
    async fn take(&self, kind: MediaKind) -> bool;

    /// What is left today, or `None` when nothing is counted.
    async fn remaining(&self, kind: MediaKind) -> Option<u32>;
}

/// The no-op budget: every generation is allowed and nothing is counted.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unmetered;

#[async_trait]
impl MediaBudget for Unmetered {
    async fn take(&self, _kind: MediaKind) -> bool {
        true
    }

    async fn remaining(&self, _kind: MediaKind) -> Option<u32> {
        None
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
