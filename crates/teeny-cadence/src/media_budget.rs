//! How much the account may generate per day (`MEDIA_IMAGES_PER_DAY`,
//! `MEDIA_VIDEOS_PER_DAY`), counted per kind because a video costs a hundred
//! times what an image does.
//!
//! A public bot can be asked "make me a video" by anybody, any number of
//! times, and a prompt asking it to be sparing is a request, not a limit.
//! This is the limit. It resets at UTC midnight and lives in memory: a
//! restart forgets the count, which errs towards generosity by at most one
//! day's cap. A port of `MediaBudget` in `legacy/src/media/store.ts`;
//! `teeny-media`'s `MediaBudget` trait is the seam it is wired in through.

use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use teeny_core::config::MediaConfig;

use crate::clock::Clock;

/// What is being generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

/// The daily caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaLimits {
    pub images_per_day: u32,
    pub videos_per_day: u32,
}

impl MediaLimits {
    pub fn from_config(config: &MediaConfig) -> Self {
        Self {
            images_per_day: config.images_per_day,
            videos_per_day: config.videos_per_day,
        }
    }
}

#[derive(Default)]
struct Used {
    day: Option<NaiveDate>,
    images: u32,
    videos: u32,
}

/// Today's generation allowance.
pub struct DailyMediaBudget {
    limits: MediaLimits,
    clock: Arc<dyn Clock>,
    used: Mutex<Used>,
}

impl DailyMediaBudget {
    pub fn new(limits: MediaLimits, clock: Arc<dyn Clock>) -> Self {
        Self {
            limits,
            clock,
            used: Mutex::new(Used::default()),
        }
    }

    /// Today's count, started over when the UTC day has turned.
    fn today(&self) -> std::sync::MutexGuard<'_, Used> {
        let today = self.clock.now().date_naive();
        let mut used = self.used.lock().expect("not poisoned");
        if used.day != Some(today) {
            *used = Used {
                day: Some(today),
                images: 0,
                videos: 0,
            };
        }
        used
    }

    fn remaining_in(&self, used: &Used, kind: MediaKind) -> u32 {
        match kind {
            MediaKind::Image => self.limits.images_per_day.saturating_sub(used.images),
            MediaKind::Video => self.limits.videos_per_day.saturating_sub(used.videos),
        }
    }

    /// What is left today.
    pub fn remaining(&self, kind: MediaKind) -> u32 {
        let used = self.today();
        self.remaining_in(&used, kind)
    }

    /// Takes one from today's allowance, or returns false when none is left.
    pub fn take(&self, kind: MediaKind) -> bool {
        let mut used = self.today();
        if self.remaining_in(&used, kind) == 0 {
            return false;
        }
        match kind {
            MediaKind::Image => used.images += 1,
            MediaKind::Video => used.videos += 1,
        }
        true
    }
}

#[cfg(test)]
#[path = "media_budget_tests.rs"]
mod tests;
