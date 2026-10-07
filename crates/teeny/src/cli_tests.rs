use clap::{CommandFactory, Parser};

use super::*;

#[test]
fn the_command_line_is_well_formed() {
    Cli::command().debug_assert();
}

#[test]
fn reads_serve() {
    let cli = Cli::try_parse_from(["teeny", "serve"]).unwrap();
    assert_eq!(cli.command, Command::Serve);
}

#[test]
fn reads_healthcheck() {
    let cli = Cli::try_parse_from(["teeny", "healthcheck"]).unwrap();
    assert_eq!(cli.command, Command::Healthcheck);
}

#[test]
fn refuses_to_guess_without_a_subcommand() {
    assert!(Cli::try_parse_from(["teeny"]).is_err());
}
