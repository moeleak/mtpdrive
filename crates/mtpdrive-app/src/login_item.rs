#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Disabled,
    Enabled,
    RequiresApproval,
    #[cfg(not(target_os = "macos"))]
    Unavailable,
}

impl Status {
    pub(crate) const fn is_registered(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn status() -> Status {
    use smappservice_rs::{AppService, ServiceType};

    map_status(AppService::new(ServiceType::MainApp).status())
}

#[cfg(not(target_os = "macos"))]
pub(crate) const fn status() -> Status {
    Status::Unavailable
}

#[cfg(target_os = "macos")]
pub(crate) fn set_enabled(enabled: bool) -> Result<Status, String> {
    use smappservice_rs::{AppService, ServiceManagementError, ServiceType};

    let service = AppService::new(ServiceType::MainApp);
    let current = map_status(service.status());
    if enabled {
        if current.is_registered() {
            return Ok(current);
        }
        match service.register() {
            Ok(()) | Err(ServiceManagementError::AlreadyRegistered) => {}
            Err(error) => return Err(error.to_string()),
        }
    } else {
        if current == Status::Disabled {
            return Ok(current);
        }
        match service.unregister() {
            Ok(()) | Err(ServiceManagementError::JobNotFound) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(map_status(service.status()))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn set_enabled(_enabled: bool) -> Result<Status, String> {
    Err("Open at Login is unavailable on this platform".to_owned())
}

#[cfg(target_os = "macos")]
pub(crate) fn open_system_settings() {
    smappservice_rs::AppService::open_system_settings_login_items();
}

#[cfg(not(target_os = "macos"))]
pub(crate) const fn open_system_settings() {}

#[cfg(target_os = "macos")]
fn map_status(status: smappservice_rs::ServiceStatus) -> Status {
    use smappservice_rs::ServiceStatus;

    match status {
        ServiceStatus::Enabled => Status::Enabled,
        ServiceStatus::RequiresApproval => Status::RequiresApproval,
        // A freshly installed main-app service can report `NotFound` before
        // its first registration even though `register` succeeds. For this
        // service type that is an off state, not a platform limitation.
        ServiceStatus::NotRegistered | ServiceStatus::NotFound => Status::Disabled,
    }
}

#[cfg(test)]
#[path = "../tests/unit/login_item.rs"]
mod tests;
