use super::*;

#[tokio::test]
async fn random_writes_and_resize_work() {
    let root = tempfile::tempdir().expect("temporary directory");
    let area = StagingArea::new(root.path(), LogStore::memory_only()).expect("staging area");
    let id = area
        .create("phone".into(), 1, 2, None, "note.txt".into())
        .await
        .expect("create");
    area.write(id, 5, &Bytes::from_static(b"world"))
        .await
        .expect("write");
    area.write(id, 0, &Bytes::from_static(b"hello"))
        .await
        .expect("write");
    assert_eq!(
        area.read(id, 0, 10).await.expect("read"),
        Bytes::from_static(b"helloworld")
    );
    area.set_len(id, 5).await.expect("truncate");
    assert_eq!(area.snapshot(id).await.expect("snapshot").size, 5);
}

#[test]
fn finder_metadata_is_recognized() {
    assert!(is_finder_metadata(".DS_Store"));
    assert!(is_finder_metadata("._photo.jpg"));
    assert!(!is_finder_metadata(".env"));
}
