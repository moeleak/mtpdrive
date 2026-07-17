use super::*;

#[test]
fn nonempty_uses_each_fallback() {
    assert_eq!(nonempty("Pixel", Some("USB"), "Unknown"), "Pixel");
    assert_eq!(nonempty("", Some("USB"), "Unknown"), "USB");
    assert_eq!(nonempty("", None, "Unknown"), "Unknown");
}

#[test]
fn image_capture_pid_parser_requires_the_exact_path_and_current_user() {
    let processes = r"
          101   501 /System/Library/Image Capture/Support/icdd
          102   502 /System/Library/Image Capture/Support/icdd
          103   501 /tmp/icdd
          104   501 /System/Library/Image Capture/Support/icdd --unexpected
        ";

    assert_eq!(image_capture_daemon_pids(processes, 501), vec![101]);
}

#[test]
fn both_mtp_root_parent_encodings_are_normalized() {
    assert_eq!(normalize_parent(ObjectHandle::ROOT.0), 0);
    assert_eq!(normalize_parent(ObjectHandle::ALL.0), 0);
    assert_eq!(normalize_parent(42), 42);
    assert_eq!(mtp_write_parent(None), Some(ObjectHandle::ALL));
    assert_eq!(mtp_write_parent(Some(42)), Some(ObjectHandle(42)));
}

fn cached_entry(handle: u64, parent: u64, name: &str, is_dir: bool) -> ObjectEntry {
    ObjectEntry {
        handle,
        storage_id: 7,
        parent,
        name: name.to_owned(),
        size: 0,
        is_dir,
        created: None,
        modified: None,
    }
}

#[test]
fn directory_listing_populates_metadata_cache() {
    let now = Instant::now();
    let mut cache = DeviceCache::default();
    let entries = vec![
        cached_entry(11, 0, "folder", true),
        cached_entry(12, 0, "photo.jpg", false),
    ];
    cache.store_listing(7, None, entries.clone(), now);

    assert_eq!(
        cache
            .listing(7, None, now)
            .expect("cached directory")
            .0
            .len(),
        2
    );
    assert_eq!(
        cache.object(7, 12).expect("cached object").name,
        "photo.jpg"
    );
}

#[test]
fn deleting_an_object_updates_its_directory_and_keeps_sibling_metadata() {
    let now = Instant::now();
    let mut cache = DeviceCache::default();
    cache.store_listing(
        7,
        None,
        vec![
            cached_entry(11, 0, "folder", true),
            cached_entry(12, 0, "photo.jpg", false),
        ],
        now,
    );
    cache.store_listing(
        7,
        Some(11),
        vec![cached_entry(13, 11, "child.txt", false)],
        now,
    );

    cache.remove_object(7, 11, now);

    assert_eq!(
        cache.listing(7, None, now).expect("cached parent").0,
        vec![cached_entry(12, 0, "photo.jpg", false)]
    );
    assert!(cache.listing(7, Some(11), now).is_none());
    assert!(cache.object(7, 11).is_none());
    assert!(cache.object(7, 13).is_none());
    assert!(cache.object(7, 12).is_some());
}

#[test]
fn stale_directory_is_returned_while_one_refresh_runs() {
    let now = Instant::now();
    let loaded_at = now - DIRECTORY_CACHE_TTL - Duration::from_millis(1);
    let mut cache = DeviceCache::default();
    cache.store_listing(
        7,
        None,
        vec![cached_entry(12, 0, "photo.jpg", false)],
        loaded_at,
    );

    let (entries, fresh) = cache.listing(7, None, now).expect("stale directory");
    assert_eq!(entries.len(), 1);
    assert!(!fresh);
    let (epoch, _cancel) = cache.begin_refresh(7, None).expect("first refresh");
    assert!(cache.begin_refresh(7, None).is_none());
    assert!(cache.finish_refresh(
        7,
        None,
        epoch,
        Some(vec![cached_entry(13, 0, "new.jpg", false)]),
        now,
    ));
    let (entries, fresh) = cache.listing(7, None, now).expect("fresh directory");
    assert!(fresh);
    assert_eq!(entries[0].name, "new.jpg");
}

#[test]
fn matching_object_count_revalidates_without_replacing_metadata() {
    let now = Instant::now();
    let loaded_at = now - DIRECTORY_CACHE_TTL - Duration::from_millis(1);
    let mut cache = DeviceCache::default();
    cache.store_listing(
        7,
        None,
        vec![cached_entry(12, 0, "photo.jpg", false)],
        loaded_at,
    );
    let (epoch, _cancel) = cache.begin_refresh(7, None).expect("background refresh");

    assert!(cache.can_count_validate(7, None, epoch, 1, now));
    assert!(cache.finish_count_validation(7, None, epoch, now));

    let (entries, fresh) = cache.listing(7, None, now).expect("validated directory");
    assert!(fresh);
    assert_eq!(entries[0].name, "photo.jpg");
}

#[test]
fn count_validation_falls_back_to_full_scan_periodically() {
    let now = Instant::now();
    let fully_loaded_at = now - DIRECTORY_FULL_REFRESH_INTERVAL;
    let mut cache = DeviceCache::default();
    cache.store_listing(
        7,
        None,
        vec![cached_entry(12, 0, "photo.jpg", false)],
        fully_loaded_at,
    );
    let (epoch, _cancel) = cache.begin_refresh(7, None).expect("background refresh");

    assert!(!cache.can_count_validate(7, None, epoch, 1, now));
}

#[test]
fn object_event_is_merged_into_a_cached_directory() {
    let now = Instant::now();
    let mut cache = DeviceCache::default();
    cache.store_listing(7, None, vec![cached_entry(12, 0, "photo.jpg", false)], now);

    cache.upsert_object(cached_entry(13, 0, "new.jpg", false), now);

    let (entries, fresh) = cache.listing(7, None, now).expect("updated directory");
    assert!(fresh);
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry.name == "new.jpg"));
}

#[test]
fn invalidation_cancels_an_in_flight_directory_refresh() {
    let now = Instant::now();
    let mut cache = DeviceCache::default();
    cache.store_listing(7, None, vec![cached_entry(12, 0, "photo.jpg", false)], now);
    let (epoch, cancel) = cache.begin_refresh(7, None).expect("background refresh");

    cache.invalidate_directory(7, None);

    assert!(cancel.is_cancelled());
    assert!(!cache.finish_refresh(
        7,
        None,
        epoch,
        Some(vec![cached_entry(13, 0, "stale.jpg", false)]),
        now,
    ));
    assert!(cache.listing(7, None, now).is_none());
}

#[test]
fn change_timestamp_is_strictly_monotonic() {
    let timestamp = AtomicU64::new(u64::MAX - 2);

    mark_timestamp_changed(&timestamp);

    assert_eq!(timestamp.load(Ordering::Acquire), u64::MAX - 1);
}
