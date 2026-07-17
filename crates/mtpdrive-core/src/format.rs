use crate::{Language, MountState};

/// Formats a byte count with binary units.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formats the current mount state using the selected language.
#[must_use]
pub fn format_mount_state(language: Language, state: &MountState) -> String {
    let strings = language.strings();
    match state {
        MountState::Unmounted => strings.unmounted.to_owned(),
        MountState::Mounting => strings.mounting.to_owned(),
        MountState::Mounted { path, .. } => language.mounted_at(path.display()),
        MountState::Error { message } => language.mount_failed(message),
    }
}
