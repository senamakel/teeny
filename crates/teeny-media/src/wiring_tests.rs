use std::sync::Arc;

use teeny_core::Config;

use super::*;

const REQUIRED: &[(&str, &str)] = &[
    ("X_BOT_USERNAME", "teenydroid"),
    ("X_API_KEY", "k"),
    ("X_API_SECRET", "s"),
    ("X_ACCESS_TOKEN", "t"),
    ("X_ACCESS_TOKEN_SECRET", "ts"),
    ("CORTEX_TOKEN", "c"),
    ("LLM_BASE_URL", "http://ladder:6969/v1"),
    ("LLM_API_KEY", "l"),
    ("LLM_MODEL", "reasoning"),
];

fn config(extra: &[(&str, &str)]) -> Config {
    let mut vars: Vec<(String, String)> = REQUIRED
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    vars.extend(
        extra
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
    );
    Config::from_vars(vars).unwrap()
}

#[test]
fn is_nothing_at_all_when_every_part_is_off() {
    assert!(build_media(&config(&[("VISION_MODEL", " ")])).is_none());
}

#[test]
fn sees_by_default_and_studies_the_artwork_only_when_it_can_also_draw() {
    let seeing = build_media(&config(&[])).unwrap();
    assert!(seeing.reader.is_some() && seeing.images.is_none() && seeing.videos.is_none());
    assert!(seeing.reference.is_none());
    assert_eq!(seeing.can_generate(), (false, false));

    let drawing = build_media(&config(&[
        ("IMAGE_GEN_ENABLED", "true"),
        ("VIDEO_GEN_ENABLED", "true"),
        ("VIDEO_GEN_API", "openai"),
    ]))
    .unwrap();
    assert_eq!(drawing.can_generate(), (true, true));
    assert!(drawing.reference.is_some());

    let no_sheet = build_media(&config(&[
        ("IMAGE_GEN_ENABLED", "true"),
        ("TINY_REFERENCE_IMAGES", "none"),
    ]))
    .unwrap();
    assert!(no_sheet.reference.is_none());

    let blind = build_media(&config(&[
        ("VISION_MODEL", " "),
        ("IMAGE_GEN_ENABLED", "true"),
    ]))
    .unwrap();
    assert!(blind.reader.is_none() && blind.images.is_some() && blind.reference.is_none());
}

#[test]
fn gates_generation_per_turn_but_never_takes_the_eyes_or_the_store() {
    let media = build_media(&config(&[
        ("IMAGE_GEN_ENABLED", "true"),
        ("VIDEO_GEN_ENABLED", "true"),
    ]))
    .unwrap();
    let images_only = media.gate(true, false);
    assert_eq!(images_only.can_generate(), (true, false));
    assert!(images_only.reference.is_some() && images_only.reader.is_some());
    assert!(Arc::ptr_eq(&images_only.store, &media.store));

    let neither = media.gate(false, false);
    assert_eq!(neither.can_generate(), (false, false));
    assert!(neither.reference.is_none() && neither.reader.is_some());
}
