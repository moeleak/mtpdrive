use super::parse_release_response;

const RELEASE: &[u8] = br#"{
    "tag_name": "v1.4.0",
    "assets": [
        {
            "name": "MTPDrive-1.4.0-universal.dmg",
            "size": 12345,
            "browser_download_url": "https://github.com/moeleak/mtpdrive/releases/download/v1.4.0/MTPDrive-1.4.0-universal.dmg"
        }
    ]
}"#;

#[test]
fn release_check_compares_semantic_versions() {
    let update = parse_release_response(RELEASE, "1.3.9").expect("valid release");
    assert!(update.update_available);
    assert_eq!(update.latest_version, "1.4.0");
    assert_eq!(
        update.asset.expect("DMG asset").name,
        "MTPDrive-1.4.0-universal.dmg"
    );

    let current = parse_release_response(RELEASE, "1.4.1").expect("valid release");
    assert!(!current.update_available);
}

#[test]
fn release_check_rejects_untrusted_urls() {
    let response = br#"{
        "tag_name": "v9.0.0",
        "assets": [{
            "name": "MTPDrive-9.0.0-universal.dmg",
            "size": 12345,
            "browser_download_url": "https://example.com/not-mtpdrive.dmg"
        }]
    }"#;
    assert!(parse_release_response(response, "1.0.0").is_err());
}

#[test]
fn release_check_rejects_unsafe_or_empty_assets() {
    for response in [
        br#"{
            "tag_name": "v9.0.0",
            "assets": [{
                "name": "../MTPDrive.dmg",
                "size": 12345,
                "browser_download_url": "https://github.com/moeleak/mtpdrive/releases/download/v9.0.0/MTPDrive.dmg"
            }]
        }"#
        .as_slice(),
        br#"{
            "tag_name": "v9.0.0",
            "assets": [{
                "name": "MTPDrive.dmg",
                "size": 0,
                "browser_download_url": "https://github.com/moeleak/mtpdrive/releases/download/v9.0.0/MTPDrive.dmg"
            }]
        }"#
        .as_slice(),
        br#"{"tag_name": "v9.0.0", "assets": []}"#.as_slice(),
    ] {
        assert!(parse_release_response(response, "1.0.0").is_err());
    }
}
