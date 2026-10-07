//! The token budget and the deterministic draws a run is replayed from.

use super::ContextBlock;
use super::text::head;

/// Four characters a token, rounded up, as the legacy estimated it.
pub fn estimate_tokens(text: &str) -> usize {
    text.encode_utf16().count().div_ceil(4)
}

/// Keeps blocks, in order, until the budget is spent. The block that crosses
/// the line is cut at a line boundary rather than dropped, so a long lore
/// file still contributes its front; anything after it is left out.
pub fn fit(blocks: &[ContextBlock], budget: usize) -> Vec<ContextBlock> {
    let mut kept = Vec::new();
    let mut spent = 0;
    for block in blocks {
        if spent + block.tokens <= budget {
            kept.push(block.clone());
            spent += block.tokens;
            continue;
        }
        let room = budget.saturating_sub(spent);
        if room < 40 {
            break;
        }
        let text = cut_at_line(&block.text, room * 4);
        if text.trim().is_empty() {
            break;
        }
        kept.push(ContextBlock {
            tokens: estimate_tokens(&text),
            text,
            ..block.clone()
        });
        break;
    }
    kept
}

fn cut_at_line(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let cut = head(text, max_chars);
    match cut.rfind('\n') {
        Some(line) if cut[..line].chars().count() > max_chars / 2 => cut[..line].to_owned(),
        _ => cut.trim_end().to_owned(),
    }
}

/// Where a draw comes from: [`Rng`] in a run, a fixed closure in a test.
pub trait Random: Send {
    /// In `[0, 1)`.
    fn next_f64(&mut self) -> f64;

    /// An index in `0..len`; 0 when `len` is 0.
    fn index(&mut self, len: usize) -> usize {
        ((self.next_f64() * len as f64) as usize).min(len.saturating_sub(1))
    }
}

impl<F: FnMut() -> f64 + Send> Random for F {
    fn next_f64(&mut self) -> f64 {
        self()
    }
}

/// A small deterministic generator (mulberry32, as the legacy used), so a
/// run can be replayed with the same draws: same memory probes, same forum
/// sample, same tone.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// A seed from the wall clock, for a run nobody asked to replay.
    pub fn seed_from_time() -> u32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        (nanos % 1_000_000) as u32
    }

}

impl Random for Rng {
    fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

pub fn shuffle<T: Clone>(items: &[T], random: &mut dyn Random) -> Vec<T> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        let j = ((random.next_f64() * (i + 1) as f64) as usize).min(i);
        out.swap(i, j);
    }
    out
}

pub fn sample<T: Clone>(items: &[T], count: usize, random: &mut dyn Random) -> Vec<T> {
    shuffle(items, random).into_iter().take(count).collect()
}

#[cfg(test)]
#[path = "tokens_tests.rs"]
mod tests;
