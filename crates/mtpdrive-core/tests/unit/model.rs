use super::ControlRequest;
use crate::{AppSettings, LanguagePreference};

#[test]
fn settings_request_round_trips_through_json() {
    let request = ControlRequest::SetSettings {
        settings: AppSettings {
            always_open_in_finder: false,
            language: LanguagePreference::SimplifiedChinese,
        },
    };
    let encoded = serde_json::to_vec(&request).expect("encode settings request");
    assert_eq!(
        serde_json::from_slice::<ControlRequest>(&encoded).expect("decode settings request"),
        request
    );
}
