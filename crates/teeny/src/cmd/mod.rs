//! The operator subcommands, one module each. Every command's work is a
//! function over its clients and an output, so a test drives it with fakes
//! and reads what it printed; `execute` wires it from the environment.

pub mod remember;
