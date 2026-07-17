use std::process::Command;

#[test]
fn help_keeps_existing_commands_and_description() {
    let output = Command::new(env!("CARGO_BIN_EXE_mtpdrive"))
        .arg("--help")
        .output()
        .expect("run mtpdrive --help");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 help output");
    assert!(stdout.starts_with("Expose Android MTP devices as a native macOS NFS volume\n"));
    for command in [
        "daemon", "status", "devices", "mount", "unmount", "open", "refresh", "logs", "shutdown",
    ] {
        assert!(stdout.contains(command), "missing {command} command");
    }
}
