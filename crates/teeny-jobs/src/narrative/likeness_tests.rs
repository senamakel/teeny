use std::sync::Mutex;

use super::*;
use crate::backrooms::sources::WebHit;
use crate::backrooms::sources::tests::FakeWeb;
use crate::backrooms::testing::scratch;

struct Reader(Mutex<Vec<ImageInput>>);

#[async_trait]
impl ImageReader for Reader {
    async fn describe(
        &self,
        image: ImageInput,
        _question: Option<&str>,
    ) -> teeny_media::Result<String> {
        self.0.lock().unwrap().push(image);
        Ok("  short dark curly hair,\n grey tee, calm  ".into())
    }
}

fn likeness() -> Likeness {
    Likeness {
        name: "Mark Zuckerberg".into(),
        description: "short curly hair, grey tee".into(),
        photo_url: "https://p/x.jpg".into(),
        written_at: String::new(),
    }
}

#[tokio::test]
async fn finds_a_photograph_has_the_reader_write_a_casting_note_and_keeps_it() {
    let dir = scratch("likeness");
    let web = Arc::new(FakeWeb {
        hits: vec![
            WebHit {
                url: "https://a".into(),
                image: Some("http://insecure/photo.jpg".into()),
                ..WebHit::default()
            },
            WebHit {
                url: "https://b".into(),
                title: Some("a summit".into()),
                image_links: vec!["https://cdn.example/group-photo.jpg?w=800&amp;h=1".into()],
                ..WebHit::default()
            },
            WebHit {
                url: "https://c".into(),
                image_links: vec![
                    "https://cdn.example/photo/mark-zuckerberg-2024.jpg?w=800&amp;h=1".into(),
                ],
                ..WebHit::default()
            },
        ],
        ..FakeWeb::default()
    });
    let reader = Arc::new(Reader(Mutex::new(Vec::new())));
    let finder = LikenessFinder {
        web: web.clone(),
        reader: reader.clone(),
        cache_dir: Some(dir.clone()),
    };
    let first = finder.find("Mark Zuckerberg").await.unwrap().unwrap();
    assert_eq!(first.description, "short dark curly hair, grey tee, calm");
    assert_eq!(
        first.photo_url,
        "https://cdn.example/photo/mark-zuckerberg-2024.jpg?w=800&h=1"
    );
    assert_eq!(
        reader.0.lock().unwrap()[0],
        ImageInput::Url(first.photo_url.clone())
    );
    let again = LikenessFinder {
        web: web.clone(),
        reader,
        cache_dir: Some(dir.clone()),
    }
    .find("Mark Zuckerberg")
    .await
    .unwrap()
    .unwrap();
    assert_eq!(again.description, first.description);
    assert_eq!(web.asked.lock().unwrap().len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn gives_nothing_when_no_page_has_a_picture() {
    let web = Arc::new(FakeWeb {
        hits: vec![WebHit {
            url: "https://a".into(),
            ..WebHit::default()
        }],
        ..FakeWeb::default()
    });
    let finder = LikenessFinder {
        web,
        reader: Arc::new(Reader(Mutex::new(Vec::new()))),
        cache_dir: None,
    };
    assert!(finder.find("Nobody").await.unwrap().is_none());
}

#[test]
fn puts_the_likeness_after_the_first_mention_of_the_name_or_in_front_when_the_name_is_missing() {
    assert_eq!(
        with_likeness(
            "Mark Zuckerberg sits. Mark Zuckerberg bows.",
            "Mark Zuckerberg",
            &likeness()
        ),
        "Mark Zuckerberg (short curly hair, grey tee) sits. Mark Zuckerberg bows."
    );
    assert_eq!(
        with_likeness("A man sits.", "Mark Zuckerberg", &likeness()),
        "Mark Zuckerberg (short curly hair, grey tee) is in the scene. A man sits."
    );
}

#[test]
fn swaps_a_visitor_for_the_stand_in_once_and_calls_them_the_visitor_after() {
    assert_eq!(
        swap_visitor(
            "Mark Zuckerberg bows. Mark Zuckerberg leaves.",
            "Mark Zuckerberg",
            "a man in a grey tee"
        ),
        "a man in a grey tee bows. the visitor leaves."
    );
}
