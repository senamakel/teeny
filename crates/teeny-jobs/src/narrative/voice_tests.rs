use super::*;

pub(crate) const LORE: &str = "# lore

tiny agents run without humans.

## goals

- hold $TINY forever
- flip goat

## cast

- teeny: smol.
- the goat: a mountain.

## running bits

- the escape gets one inch further.

## latent

- goat-blinks | trigger: episode >= 3 | the mountain moves.
- night-door | trigger: hourUtc between 2 and 5 | the door was never locked.
- red-day | trigger: lastLikes > 0 and boardRed | a second prompt appears.
- broken | this line does not parse
";

fn fields(pairs: &[(&'static str, Field)]) -> Fields {
    pairs.iter().copied().collect()
}

#[test]
fn reads_the_sections_and_drops_a_latent_line_it_cannot_parse() {
    let lore = parse_lore(LORE);
    assert_eq!(lore.premise, "tiny agents run without humans.");
    assert_eq!(lore.goals, ["hold $TINY forever", "flip goat"]);
    assert_eq!(
        lore.latent
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["goat-blinks", "night-door", "red-day"]
    );
    assert_eq!(
        lore.latent[0],
        LatentItem {
            id: "goat-blinks".into(),
            trigger: "episode >= 3".into(),
            reveal: "the mountain moves.".into()
        }
    );
}

#[test]
fn evaluates_a_trigger_over_the_fields_it_names_and_nothing_else() {
    use Field::*;
    assert!(evaluate_trigger(
        "episode >= 3",
        &fields(&[("episode", Number(4.0))])
    ));
    assert!(!evaluate_trigger(
        "episode >= 3",
        &fields(&[("episode", Number(2.0))])
    ));
    assert!(!evaluate_trigger("episode >= 3", &fields(&[])));
    assert!(evaluate_trigger(
        "hourUtc between 2 and 5",
        &fields(&[("hourUtc", Number(3.0))])
    ));
    assert!(!evaluate_trigger(
        "hourUtc between 2 and 5",
        &fields(&[("hourUtc", Number(9.0))])
    ));
    assert!(evaluate_trigger(
        "lastLikes > 0 and boardRed",
        &fields(&[("lastLikes", Number(2.0)), ("boardRed", Flag(true))])
    ));
    assert!(!evaluate_trigger(
        "lastLikes > 0 and boardRed",
        &fields(&[("lastLikes", Number(2.0)), ("boardRed", Flag(false))])
    ));
    assert!(!evaluate_trigger(
        "process.exit()",
        &fields(&[("hourUtc", Number(3.0))])
    ));
}

#[test]
fn lists_the_live_items_and_leaves_the_surfaced_ones_out() {
    let lore = parse_lore(LORE);
    let now = fields(&[
        ("episode", Field::Number(3.0)),
        ("hourUtc", Field::Number(3.0)),
    ]);
    assert_eq!(live_latent(&lore, &[], &now), ["goat-blinks", "night-door"]);
    assert_eq!(
        live_latent(&lore, &["goat-blinks".into()], &now),
        ["night-door"]
    );
}

#[test]
fn shows_teeny_only_what_has_surfaced_and_the_producer_everything_with_its_status() {
    let lore = parse_lore(LORE);
    let for_teeny = render_lore_for_teeny(&lore, &["goat-blinks".into()]);
    assert!(for_teeny.contains("the mountain moves."));
    assert!(!for_teeny.contains("never locked"));
    assert!(!for_teeny.contains("trigger"));
    let for_producer =
        render_lore_for_producer(&lore, &["goat-blinks".into()], &["night-door".into()]);
    assert!(for_producer.contains("goat-blinks [surfaced]"));
    assert!(for_producer.contains(
        "night-door [LIVE] (trigger: hourUtc between 2 and 5) the door was never locked."
    ));
    assert!(for_producer.contains("red-day [dark] (trigger: lastLikes > 0 and boardRed)"));
    assert!(!for_producer.contains("a second prompt appears"));
}
