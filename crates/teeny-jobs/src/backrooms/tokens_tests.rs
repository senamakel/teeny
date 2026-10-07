use super::*;
use crate::backrooms::{BlockKind, ContextBlock};

fn block(kind: BlockKind, label: &str, text: String, tokens: usize) -> ContextBlock {
    ContextBlock {
        kind,
        label: label.into(),
        text,
        tokens,
    }
}

#[test]
fn keeps_whole_blocks_until_the_budget_is_spent_and_cuts_the_last_one_at_a_line() {
    let blocks = vec![
        block(BlockKind::Seed, "a", "x".repeat(400), 100),
        block(
            BlockKind::Lore,
            "b",
            format!("{}\n{}\n{}", "y".repeat(200), "z".repeat(200), "w".repeat(200)),
            151,
        ),
        block(BlockKind::Past, "c", "never".into(), 2),
    ];
    let kept = fit(&blocks, 190);
    assert_eq!(
        kept.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(kept[1].text, "y".repeat(200));
    assert!(kept.iter().map(|b| b.tokens).sum::<usize>() <= 190);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
}

#[test]
fn draws_the_same_numbers_from_the_same_seed() {
    let mut a = Rng::new(7);
    let mut b = Rng::new(7);
    let first: Vec<f64> = (0..3).map(|_| a.next_f64()).collect();
    let second: Vec<f64> = (0..3).map(|_| b.next_f64()).collect();
    assert_eq!(first, second);
    assert!(first.iter().all(|value| (0.0..1.0).contains(value)));
    assert_ne!(
        shuffle(&[1, 2, 3, 4, 5], &mut Rng::new(1)),
        shuffle(&[1, 2, 3, 4, 5], &mut Rng::new(2))
    );
    let mut sorted = shuffle(&[1, 2, 3, 4, 5], &mut Rng::new(3));
    sorted.sort_unstable();
    assert_eq!(sorted, [1, 2, 3, 4, 5]);
    assert_eq!(sample(&[1, 2, 3, 4, 5], 2, &mut Rng::new(3)).len(), 2);
}

#[test]
fn matches_the_legacy_mulberry32_stream() {
    // rng(1)() in the TypeScript service.
    let mut random = Rng::new(1);
    assert!((random.next_f64() - 0.627_073_940_588_161_3).abs() < 1e-12);
    assert!((random.next_f64() - 0.002_735_721_180_215_478).abs() < 1e-12);
}
