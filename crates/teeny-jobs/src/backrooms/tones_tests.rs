use super::*;
use crate::backrooms::tokens::Rng;

#[test]
fn picks_a_tone_and_mostly_stays_on_it() {
    let mut random = Rng::new(3);
    let first = pick_tone(&mut random);
    assert!(TONES.contains(&first));
    let mut moved = 0;
    let mut current = first;
    for _ in 0..200 {
        let next = drift_tone(&mut random, current);
        if next != current {
            moved += 1;
        }
        current = next;
    }
    assert!(moved > 40, "moved {moved}");
    assert!(moved < 120, "moved {moved}");
}
