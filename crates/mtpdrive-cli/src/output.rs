use anyhow::Result;
use mtpdrive_core::{
    DeviceSummary, Language, LogRecord, ServiceSnapshot, format_bytes, format_mount_state,
};
use std::io::Write;

pub(crate) fn status(
    output: &mut dyn Write,
    language: Language,
    snapshot: &ServiceSnapshot,
) -> Result<()> {
    let strings = language.strings();
    writeln!(output, "MTPDrive {}", snapshot.version)?;
    writeln!(
        output,
        "{}: {}",
        strings.mount_label,
        format_mount_state(language, &snapshot.mount)
    )?;
    writeln!(
        output,
        "{}: {}",
        strings.devices_label,
        snapshot.devices.len()
    )?;
    if let Some(error) = &snapshot.last_error {
        writeln!(output, "{}: {error}", strings.last_error_label)?;
    }
    Ok(())
}

pub(crate) fn devices(
    output: &mut dyn Write,
    language: Language,
    devices: &[DeviceSummary],
) -> Result<()> {
    let strings = language.strings();
    if devices.is_empty() {
        writeln!(output, "{}", strings.no_mtp_devices)?;
        return Ok(());
    }
    for device in devices {
        writeln!(
            output,
            "{} {}  {}={}  {}={}",
            device.manufacturer,
            device.model,
            strings.serial_label,
            device.serial,
            strings.writable_label,
            localized_bool(language, device.writable)
        )?;
        for storage in &device.storages {
            writeln!(
                output,
                "  {}  {}={}  {}={}  {}={}",
                storage.name,
                strings.free_label,
                format_bytes(storage.free_bytes),
                strings.total_label,
                format_bytes(storage.total_bytes),
                strings.writable_label,
                localized_bool(language, storage.writable)
            )?;
        }
    }
    Ok(())
}

pub(crate) fn log(output: &mut dyn Write, record: &LogRecord, json: bool) -> Result<()> {
    if json {
        writeln!(output, "{}", serde_json::to_string(record)?)?;
    } else {
        writeln!(
            output,
            "{} {:?} {:<10} {}",
            record.unix_millis, record.level, record.target, record.message
        )?;
    }
    Ok(())
}

fn localized_bool(language: Language, value: bool) -> &'static str {
    let strings = language.strings();
    if value { strings.yes } else { strings.no }
}

#[cfg(test)]
#[path = "../tests/unit/output.rs"]
mod tests;
