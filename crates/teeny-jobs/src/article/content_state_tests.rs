use super::*;

fn kinds(state: &ContentState) -> Vec<(BlockType, String)> {
    state
        .blocks
        .iter()
        .map(|b| (b.kind, b.text.clone()))
        .collect()
}

fn range(offset: u32, length: u32, style: Style) -> InlineStyle {
    InlineStyle {
        offset,
        length,
        style,
    }
}

#[test]
fn turns_headings_paragraphs_lists_and_quotes_into_typed_blocks() {
    let markdown = [
        "# the title is not a block",
        "",
        "the dek, on one line.",
        "",
        "## the kettle",
        "",
        "first line of a paragraph",
        "second line of the same paragraph.",
        "",
        "### a smaller heading",
        "",
        "- one thing",
        "- two things",
        "",
        "1. first",
        "2) second",
        "",
        "> the terminal said",
        "> something else",
        "",
        "---",
        "",
        "the end.",
    ]
    .join("\n");
    let out = markdown_to_content_state(&markdown, &BTreeMap::new());
    use BlockType::*;
    assert_eq!(
        kinds(&out),
        [
            (Unstyled, "the dek, on one line.".into()),
            (HeaderTwo, "the kettle".into()),
            (
                Unstyled,
                "first line of a paragraph second line of the same paragraph.".into()
            ),
            (HeaderThree, "a smaller heading".into()),
            (UnorderedListItem, "one thing".into()),
            (UnorderedListItem, "two things".into()),
            (OrderedListItem, "first".into()),
            (OrderedListItem, "second".into()),
            (Blockquote, "the terminal said\nsomething else".into()),
            (Atomic, " ".into()),
            (Unstyled, "the end.".into()),
        ]
    );
    assert_eq!(out.entities.len(), 1);
    assert_eq!(out.entities[0].value.kind, EntityKind::Divider);
    assert_eq!(
        out.blocks[9].entity_ranges,
        [EntityRange {
            key: 0,
            offset: 0,
            length: 1
        }]
    );
}

#[test]
fn places_a_picture_as_an_atomic_block_and_drops_one_that_was_never_drawn() {
    let markdown = [
        "## one",
        "",
        "![a kettle at night](image:s1)",
        "",
        "text.",
        "",
        "## two",
        "",
        "![lost](image:s2)",
        "",
        "more.",
    ]
    .join("\n");
    let images = BTreeMap::from([(
        "s1".to_owned(),
        ImageRef {
            media_id: "m-1".into(),
            caption: Some("the kettle".into()),
        },
    )]);
    let out = markdown_to_content_state(&markdown, &images);
    use BlockType::*;
    assert_eq!(
        out.blocks.iter().map(|b| b.kind).collect::<Vec<_>>(),
        [HeaderTwo, Atomic, Unstyled, HeaderTwo, Unstyled]
    );
    let data = &out.entities[0].value.data;
    assert_eq!(data.media_items[0].media_id, "m-1");
    assert_eq!(data.caption.as_deref(), Some("the kettle"));
    let shaped = record_shape(&out);
    assert_eq!(
        shaped["entities"][0]["value"]["data"]["mediaItems"][0]["mediaCategory"],
        "tweet_image"
    );
    assert_eq!(shaped["blocks"][1]["entityRanges"][0]["key"], 0);
}

#[test]
fn uses_the_placeholder_caption_when_the_image_has_none() {
    let images = BTreeMap::from([(
        "s1".to_owned(),
        ImageRef {
            media_id: "m-1".into(),
            caption: None,
        },
    )]);
    let out = markdown_to_content_state("![from the markdown](image:s1)", &images);
    assert_eq!(
        out.entities[0].value.data.caption.as_deref(),
        Some("from the markdown")
    );
}

#[test]
fn drops_fenced_code_and_handles_the_empty_string() {
    let out = markdown_to_content_state(
        &["before", "", "```sh", "rm -rf /", "```", "", "after"].join("\n"),
        &BTreeMap::new(),
    );
    assert_eq!(
        out.blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>(),
        ["before", "after"]
    );
    assert!(out.entities.is_empty());
    let empty = markdown_to_content_state("", &BTreeMap::new());
    assert!(empty.blocks.is_empty() && empty.entities.is_empty());
}

#[test]
fn strips_the_markers_and_records_ranges_over_the_stripped_text() {
    let (text, ranges) = parse_inline("a **bold** word, an _italic_ one, and ~~gone~~.");
    assert_eq!(text, "a bold word, an italic one, and gone.");
    assert_eq!(
        ranges,
        [
            range(2, 4, Style::Bold),
            range(16, 6, Style::Italic),
            range(32, 4, Style::Strikethrough)
        ]
    );
}

#[test]
fn counts_offsets_in_utf16_code_units_as_x_does() {
    let (text, ranges) = parse_inline("🙂 **yes**");
    assert_eq!(text, "🙂 yes");
    assert_eq!(ranges, [range(3, 3, Style::Bold)]);
}

#[test]
fn leaves_underscores_inside_words_and_lone_asterisks_alone_and_puts_back_an_unclosed_marker() {
    let (text, ranges) = parse_inline("the fridge_rate file and 2 * 3");
    assert_eq!(text, "the fridge_rate file and 2 * 3");
    assert!(ranges.is_empty());
    let (text, ranges) = parse_inline("**not closed, just loud");
    assert_eq!(text, "**not closed, just loud");
    assert!(ranges.is_empty());
}

#[test]
fn nests_bold_inside_italic() {
    let (text, ranges) = parse_inline("_one **two** three_");
    assert_eq!(text, "one two three");
    assert_eq!(
        ranges,
        [range(0, 13, Style::Italic), range(4, 3, Style::Bold)]
    );
}
