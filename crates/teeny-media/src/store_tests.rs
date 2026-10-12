use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use super::*;

fn media(kind: MediaKind, byte: u8, mime_type: MimeType) -> NewMedia {
    NewMedia {
        kind,
        data: vec![byte],
        mime_type,
        prompt: format!("prompt {byte}"),
    }
}

fn clock(start: i64) -> (Arc<AtomicI64>, impl Fn() -> i64 + Send + Sync + 'static) {
    let now = Arc::new(AtomicI64::new(start));
    let reader = now.clone();
    (now, move || reader.load(Ordering::SeqCst))
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("teeny-media-store-{name}-{}", std::process::id()))
}

#[test]
fn hands_out_ids_by_kind_keeps_the_bytes_and_forgets_them_after_the_ttl() {
    let (now, read) = clock(1_000_000);
    let store = MediaStore::new()
        .with_ttl(Duration::from_secs(60))
        .with_clock(read);
    let image = store.put(media(MediaKind::Image, 1, MimeType::Png));
    let video = store.put(media(MediaKind::Video, 2, MimeType::Mp4));

    // 1_000_000 in base 36, then the counter.
    assert_eq!(image.id, "img_lfls_1");
    assert_eq!(video.id, "vid_lfls_2");
    assert!(is_media_id(&image.id) && !is_media_id("img_LFLS_1") && !is_media_id("gif_lfls_1"));
    assert_eq!(store.get(&image.id).unwrap().data, vec![1]);
    assert_eq!(image.created_at, 1_000_000);

    now.fetch_add(61_000, Ordering::SeqCst);
    assert!(store.get(&image.id).is_none());
    assert!(store.list().is_empty());
}

#[test]
fn drops_the_oldest_when_full() {
    let store = MediaStore::new().with_max_items(2);
    let first = store.put(media(MediaKind::Image, 1, MimeType::Png));
    store.put(media(MediaKind::Image, 2, MimeType::Png));
    // As the legacy store did: a full store makes room on every read too.
    assert!(store.get(&first.id).is_none());
    assert_eq!(store.list().len(), 1);
}

#[test]
fn writes_a_copy_named_by_id_and_finds_it_again_after_a_restart() {
    let dir = scratch("disk");
    let (_, read) = clock(1_700_000_000_000);
    let store = MediaStore::new().with_output_dir(&dir).with_clock(read);
    let stored = store.put(media(MediaKind::Image, 7, MimeType::Jpeg));
    let path = stored.path.clone().unwrap();
    assert_eq!(path, dir.join(format!("{}.jpg", stored.id)));
    assert_eq!(std::fs::read(&path).unwrap(), vec![7]);

    // A fresh process, an hour minus a minute later: the file is the media.
    let (now, read) = clock(1_700_000_000_000 + 59 * 60_000);
    let restarted = MediaStore::new().with_output_dir(&dir).with_clock(read);
    let found = restarted.get(&stored.id).unwrap();
    assert_eq!(
        (found.data, found.mime_type, found.kind),
        (vec![7], MimeType::Jpeg, MediaKind::Image)
    );
    assert_eq!(found.created_at, 1_700_000_000_000);
    assert!(restarted.get("img_../../etc/passwd_1").is_none());

    // Past the TTL the copy is history, not something to attach.
    now.fetch_add(2 * 60_000, Ordering::SeqCst);
    assert!(restarted.get(&stored.id).is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn keeps_the_media_when_the_copy_cannot_be_written() {
    let blocker = scratch("blocker");
    std::fs::write(&blocker, b"a file where the directory should be").unwrap();
    let store = MediaStore::new().with_output_dir(blocker.join("media"));
    let stored = store.put(media(MediaKind::Video, 3, MimeType::Mp4));
    assert!(stored.path.is_none());
    assert_eq!(store.get(&stored.id).unwrap().data, vec![3]);
    std::fs::remove_file(blocker).unwrap();
}

#[tokio::test]
async fn an_unmetered_budget_always_says_yes_and_counts_nothing() {
    let budget = Unmetered;
    assert!(budget.take(MediaKind::Video).await);
    assert_eq!(budget.remaining(MediaKind::Image).await, None);
}
