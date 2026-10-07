use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

/// A vision reader that answers from a script, one entry per call, and
/// remembers what it was shown.
#[derive(Default)]
struct ScriptedReader {
    answers: Mutex<Vec<std::result::Result<String, String>>>,
    seen: Mutex<Vec<(ImageInput, Option<String>)>>,
}

impl ScriptedReader {
    fn answering(answers: Vec<std::result::Result<&str, &str>>) -> Arc<Self> {
        let answers = answers.into_iter().rev().map(|answer| answer.map(str::to_owned).map_err(str::to_owned)).collect();
        Arc::new(Self { answers: Mutex::new(answers), seen: Mutex::default() })
    }

    fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait]
impl ImageReader for ScriptedReader {
    async fn describe(&self, image: ImageInput, question: Option<&str>) -> Result<String> {
        let size = match &image {
            ImageInput::Bytes { data, .. } => data.len(),
            ImageInput::Url(_) => 0,
        };
        self.seen.lock().unwrap().push((image, question.map(str::to_owned)));
        match self.answers.lock().unwrap().pop() {
            Some(Ok(answer)) => Ok(answer),
            Some(Err(error)) => Err(Error::failed(error)),
            None => Ok(format!("look {size}")),
        }
    }
}

struct FixedText {
    answer: String,
    prompts: Mutex<Vec<(Option<String>, String)>>,
    calls: AtomicUsize,
}

impl FixedText {
    fn new(answer: &str) -> Arc<Self> {
        Arc::new(Self { answer: answer.into(), prompts: Mutex::default(), calls: AtomicUsize::new(0) })
    }
}

#[async_trait]
impl TextModel for FixedText {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push((system.map(str::to_owned), prompt.to_owned()));
        Ok(self.answer.clone())
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("teeny-media-ref-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), PNG).unwrap();
        std::fs::write(dir.join("b.png"), PNG).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const SHEET: &str = "Tiny is a small round robot with a cream shell and teal eyes.";

#[test]
fn ships_with_the_vendored_artwork_every_file_present() {
    assert_eq!(DEFAULT_REFERENCE_IMAGES.len(), 10);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for relative in DEFAULT_REFERENCE_IMAGES {
        let path = root.join(relative);
        assert!(std::fs::metadata(&path).unwrap().len() > 10_000, "{relative}");
        let input = reference_input(path.to_str().unwrap()).unwrap();
        assert!(matches!(input, ImageInput::Bytes { mime_type: MimeType::Jpeg, .. }), "{relative}");
    }
    assert_eq!(reference_input("https://example.com/x.jpg").unwrap(), ImageInput::Url("https://example.com/x.jpg".into()));
    assert_eq!(reference_input("HTTP://example.com/x.jpg").unwrap(), ImageInput::Url("HTTP://example.com/x.jpg".into()));
}

#[test]
fn reads_the_type_from_the_extension_when_the_bytes_do_not_say() {
    let scratch = Scratch::new("ext");
    std::fs::write(scratch.0.join("odd.webp"), b"short").unwrap();
    std::fs::write(scratch.0.join("odd.txt"), b"short").unwrap();
    let input = reference_input(&scratch.file("odd.webp")).unwrap();
    assert!(matches!(input, ImageInput::Bytes { mime_type: MimeType::Webp, .. }));
    let error = reference_input(&scratch.file("odd.txt")).unwrap_err();
    assert!(error.to_string().ends_with("odd.txt is not an image this can read"), "{error}");
}

#[tokio::test]
async fn studies_each_picture_distils_one_sheet_caches_it_and_reads_the_cache_next_time() {
    let scratch = Scratch::new("cache");
    let (a, b) = (scratch.file("a.png"), scratch.file("b.png"));
    let cache = scratch.0.join("sheet.json");
    let reader = ScriptedReader::answering(vec![Ok("a round cream robot"), Ok("a round cream robot")]);
    let text = FixedText::new(SHEET);
    let sheet = ReferenceSheet::new(vec![a.clone(), b.clone(), a.clone()], reader.clone(), text.clone())
        .with_cache_path(&cache);

    assert_eq!(sheet.get().await.as_deref(), Some(SHEET));
    // Deduplicated, and the reader saw bytes, not paths.
    assert_eq!(reader.calls(), 2);
    let (input, question) = reader.seen.lock().unwrap()[0].clone();
    assert_eq!(input, ImageInput::Bytes { data: PNG.to_vec(), mime_type: MimeType::Png });
    assert!(question.unwrap().contains("illustrator"));
    let (system, prompt) = text.prompts.lock().unwrap()[0].clone();
    assert_eq!(system.as_deref(), Some("You write terse, concrete visual briefs for image models."));
    assert!(prompt.contains("PICTURE 1:\na round cream robot"));
    assert!(prompt.contains("PICTURE 2:"));
    // Built once, then remembered.
    assert_eq!(sheet.get().await.as_deref(), Some(SHEET));
    assert_eq!(text.calls.load(Ordering::SeqCst), 1);

    let cached: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
    assert_eq!(cached["urls"], json!([a, b]));
    assert_eq!(cached["sheet"], SHEET);
    assert_eq!(cached["looks"][&a], "a round cream robot");
    assert!(cached["createdAt"].as_str().unwrap().ends_with('Z'));

    let blind = ScriptedReader::answering(vec![]);
    let again = ReferenceSheet::new(vec![b.clone(), a.clone()], blind.clone(), FixedText::new("unused")).with_cache_path(&cache);
    assert_eq!(again.get().await.as_deref(), Some(SHEET));
    assert_eq!(blind.calls(), 0);

    // A different list is a different sheet.
    let changed = ReferenceSheet::new(vec![a.clone()], reader.clone(), FixedText::new("Tiny is different."))
        .with_cache_path(&cache);
    assert_eq!(changed.get().await.as_deref(), Some("Tiny is different."));
}

#[tokio::test]
async fn reads_a_cache_the_legacy_service_wrote() {
    let scratch = Scratch::new("legacy");
    let cache = scratch.0.join("reference-sheet.json");
    let legacy = json!({
        "urls": ["assets/tiny/a.jpg", "assets/tiny/b.jpg"],
        "sheet": "Tiny is a robot.",
        "looks": { "assets/tiny/a.jpg": "a look", "assets/tiny/b.jpg": "b look" },
        "createdAt": "2026-09-14T07:00:00.000Z"
    });
    std::fs::write(&cache, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();
    let reader = ScriptedReader::answering(vec![]);
    let sheet = ReferenceSheet::new(vec!["assets/tiny/b.jpg".into(), "assets/tiny/a.jpg".into()], reader.clone(), FixedText::new("x"))
        .with_cache_path(&cache);
    assert_eq!(sheet.get().await.as_deref(), Some("Tiny is a robot."));
    assert_eq!(sheet.seed(0.999).await, Some(ReferenceSeed { url: "assets/tiny/b.jpg".into(), look: "b look".into() }));
    assert_eq!(reader.calls(), 0);
}

#[tokio::test]
async fn builds_from_the_pictures_it_could_read_and_is_nothing_when_it_could_read_none() {
    let scratch = Scratch::new("flaky");
    let flaky = ScriptedReader::answering(vec![Err("403"), Ok("a robot")]);
    let sheet = ReferenceSheet::new(vec![scratch.file("a.png"), scratch.file("b.png")], flaky, FixedText::new(SHEET));
    assert!(sheet.get().await.unwrap().starts_with("Tiny is"));

    let blind = ScriptedReader::answering(vec![Err("403"), Err("403")]);
    let none = ReferenceSheet::new(vec![scratch.file("a.png")], blind.clone(), FixedText::new(SHEET));
    assert_eq!(none.get().await, None);
    // Not remembered as a failure: the next call tries again.
    assert_eq!(none.get().await, None);
    assert_eq!(blind.calls(), 2);

    let missing = ReferenceSheet::new(vec![scratch.file("gone.png")], ScriptedReader::answering(vec![]), FixedText::new(SHEET));
    assert_eq!(missing.get().await, None);
}

#[tokio::test]
async fn hands_out_one_reference_picture_with_its_own_description_chosen_by_the_random_given() {
    let scratch = Scratch::new("seed");
    let (a, b) = (scratch.file("a.png"), scratch.file("b.png"));
    let cache = scratch.0.join("sheet.json");
    let sheet = ReferenceSheet::new(vec![a.clone(), b.clone()], ScriptedReader::answering(vec![]), FixedText::new("Tiny is a robot."))
        .with_cache_path(&cache);
    let first = sheet.seed(0.0).await.unwrap();
    assert_eq!(first, ReferenceSeed { url: a.clone(), look: "look 12".into() });
    assert_eq!(sheet.seed(0.999).await.unwrap().url, b);

    // The looks come back from the cache too, so a restart still has seeds.
    let again = ReferenceSheet::new(vec![a, b], ScriptedReader::answering(vec![]), FixedText::new("x")).with_cache_path(&cache);
    assert_eq!(again.seed(0.0).await, Some(first));

    let none = ReferenceSheet::new(Vec::new(), ScriptedReader::answering(vec![]), FixedText::new("x"));
    assert_eq!(none.get().await, None);
    assert_eq!(none.seed(0.5).await, None);
}

#[test]
fn goes_in_front_of_the_scene() {
    assert_eq!(with_reference(Some("Tiny is a robot."), "Tiny on a beach"), "Tiny is a robot.\n\nScene: Tiny on a beach");
    assert_eq!(with_reference(None, "Tiny on a beach"), "Tiny on a beach");
}

#[test]
fn puts_the_seed_between_the_sheet_and_the_scene() {
    let seed = ReferenceSeed { url: "x".into(), look: "a beach".into() };
    assert_eq!(
        with_seed(Some("Tiny is a robot."), Some(&seed), "Tiny codes", None),
        "Tiny is a robot.\n\nStyle reference, one of Tiny's official pictures (match its rendering, palette and mood, not its scene): a beach\n\nScene: Tiny codes"
    );
    assert_eq!(with_seed(Some("Tiny is a robot."), None, "Tiny codes", None), "Tiny is a robot.\n\nScene: Tiny codes");
    assert!(!with_seed(None, Some(&seed), "Tiny codes", None).contains("Tiny is"));
}

#[test]
fn drops_the_seed_first_when_the_prompt_is_too_long_and_cuts_the_scene_last() {
    let sheet = "Tiny is a robot. ".repeat(20).trim().to_owned();
    let seed = ReferenceSeed { url: "x".into(), look: "y".repeat(500) };
    let scene = "Tiny codes at dawn. ".repeat(30).trim().to_owned();
    let fitted = with_seed(Some(&sheet), Some(&seed), &scene, Some(1_000));
    assert!(fitted.len() <= 1_000);
    assert!(!fitted.contains("Style reference"));
    assert!(fitted.starts_with("Tiny is a robot.") && fitted.contains("\n\nScene: Tiny codes at dawn.") && fitted.ends_with('.'));
    assert!(with_seed(Some(&sheet), Some(&seed), "short", Some(2_000)).contains("Style reference"));
}

#[tokio::test]
async fn the_text_model_asks_the_chat_endpoint_and_surfaces_its_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [{ "message": { "content": "  Tiny is.  " } }] })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({ "error": { "message": "slow down" } })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "choices": [] })))
        .mount(&server)
        .await;
    let model = OpenAiTextModel::new(format!("{}/v1/", server.uri()), "k", "chat");
    assert_eq!(model.complete(Some("be brief"), "describe").await.unwrap(), "Tiny is.");
    let body: serde_json::Value = server.received_requests().await.unwrap()[0].body_json().unwrap();
    assert_eq!(
        body,
        json!({ "model": "chat", "temperature": 0.2, "messages": [
            { "role": "system", "content": "be brief" }, { "role": "user", "content": "describe" }
        ] })
    );
    assert_eq!(model.complete(None, "x").await.unwrap_err().to_string(), "text request failed (429): slow down");
    assert_eq!(model.complete(None, "x").await.unwrap_err().to_string(), "the text model returned nothing");
}
