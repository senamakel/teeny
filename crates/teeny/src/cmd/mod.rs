//! The operator subcommands, one module each. Every command's work is a
//! function over its clients and an output, so a test drives it with fakes
//! and reads what it printed; `execute` wires it from the environment.

pub mod auth_x;
pub mod cron;
pub mod dry_run;
pub mod forget;
pub mod learnings;
pub mod memory;
pub mod redteam;
pub mod remember;
pub mod unsay;
pub mod webhook;

#[cfg(test)]
mod test_support;

use teeny_core::Config;

/// `config` as a rehearsal runs it: `DRY_RUN` on, whatever the environment
/// says.
pub fn forced_dry_run(mut config: Config) -> Config {
    config.dry_run = true;
    config
}
