use super::{Cli, Command};
use clap::Parser;

#[test]
fn status_is_the_default_command() {
    let cli = Cli::try_parse_from(["mtpdrive"]).expect("default arguments");
    assert!(cli.command.is_none());
}

#[test]
fn log_flags_preserve_the_existing_cli_shape() {
    let cli = Cli::try_parse_from(["mtpdrive", "logs", "--follow", "--json", "-n", "42"])
        .expect("log arguments");
    assert!(matches!(
        cli.command,
        Some(Command::Logs {
            follow: true,
            json: true,
            limit: 42
        })
    ));
}
