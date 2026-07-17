use super::Status;

#[test]
fn only_registered_states_enable_the_toggle() {
    assert!(!Status::Disabled.is_registered());
    assert!(Status::Enabled.is_registered());
    assert!(Status::RequiresApproval.is_registered());
    #[cfg(not(target_os = "macos"))]
    assert!(!Status::Unavailable.is_registered());
}

#[cfg(target_os = "macos")]
#[test]
fn service_management_statuses_are_normalized_for_the_main_app() {
    use super::map_status;
    use smappservice_rs::ServiceStatus;

    assert_eq!(map_status(ServiceStatus::NotRegistered), Status::Disabled);
    assert_eq!(map_status(ServiceStatus::Enabled), Status::Enabled);
    assert_eq!(
        map_status(ServiceStatus::RequiresApproval),
        Status::RequiresApproval
    );
    assert_eq!(map_status(ServiceStatus::NotFound), Status::Disabled);
}
