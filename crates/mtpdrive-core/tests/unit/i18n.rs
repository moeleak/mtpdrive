use super::{Language, detect_language_from, parse_language, parse_language_list};

#[test]
fn parses_supported_language_identifiers() {
    assert_eq!(parse_language("en-US"), Some(Language::English));
    assert_eq!(
        parse_language("zh_Hans_CN.UTF-8"),
        Some(Language::SimplifiedChinese)
    );
    assert_eq!(parse_language("zh-CN"), Some(Language::SimplifiedChinese));
}

#[test]
fn does_not_claim_traditional_chinese() {
    assert_eq!(parse_language("zh-Hant"), None);
    assert_eq!(parse_language("zh-TW"), None);
}

#[test]
fn honors_the_first_supported_macos_language() {
    let preferences = "(\n    \"de-US\",\n    \"zh-Hans-US\",\n    \"en-US\"\n)";
    assert_eq!(
        parse_language_list(preferences),
        Some(Language::SimplifiedChinese)
    );
    assert_eq!(
        detect_language_from(&["en-US", "zh-Hans-US"]),
        Language::English
    );
}

#[test]
fn falls_back_to_english() {
    assert_eq!(detect_language_from(&["de-DE", "fr-FR"]), Language::English);
}
