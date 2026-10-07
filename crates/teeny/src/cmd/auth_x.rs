//! `teeny auth-x`: the X OAuth 1.0a three-legged flow, once, by PIN. It
//! mints the account's access pair (`X_ACCESS_TOKEN`,
//! `X_ACCESS_TOKEN_SECRET`); the service cannot start without it, because
//! every write is signed with it.
//!
//! The pair is a credential: it is either written into an env file
//! (`--env-file`, mode 0600, never printed) or printed exactly once with a
//! warning, never logged.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use teeny_core::Secret;
use teeny_x::auth::{ThreeLegged, authorize_url};

/// `teeny auth-x [--env-file .env]`
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct AuthXArgs {
    /// Write the pair into this env file (keys rewritten in place, mode
    /// 0600) instead of printing it.
    #[arg(long, value_name = "PATH")]
    pub env_file: Option<PathBuf>,
}

pub async fn run(
    flow: &ThreeLegged,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    env_file: Option<&Path>,
) -> anyhow::Result<()> {
    let request = flow.request_token("oob").await?;
    writeln!(
        out,
        "\nOpen this URL and authorize the app as the bot account:\n"
    )?;
    writeln!(out, "  {}\n", authorize_url(&request))?;
    write!(out, "Paste the PIN shown by X: ")?;
    out.flush()?;
    let mut pin = String::new();
    input.read_line(&mut pin)?;
    let pin = pin.trim();
    if pin.is_empty() {
        bail!("no PIN was entered; run teeny auth-x again");
    }

    let access = flow.access_token(&request, pin).await?;
    let account = access
        .screen_name
        .as_deref()
        .map(|name| format!("@{name}"))
        .unwrap_or_else(|| "the account".to_owned());
    match env_file {
        Some(path) => {
            let contents = match std::fs::read_to_string(path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(error) => return Err(error).context(format!("reading {}", path.display())),
            };
            let contents = write_env_value(&contents, "X_ACCESS_TOKEN", &access.token);
            let contents =
                write_env_value(&contents, "X_ACCESS_TOKEN_SECRET", access.secret.expose());
            write_private(path, &contents)?;
            writeln!(
                out,
                "\nWrote X_ACCESS_TOKEN and X_ACCESS_TOKEN_SECRET for {account} to {}.",
                path.display()
            )?;
        }
        None => {
            writeln!(
                out,
                "\nWARNING: these are {account}'s credentials, shown once and not stored anywhere."
            )?;
            writeln!(
                out,
                "Anyone holding them can post as the account. Put them in the service's"
            )?;
            writeln!(
                out,
                "environment now, then clear this terminal and its scrollback.\n"
            )?;
            writeln!(out, "X_ACCESS_TOKEN={}", access.token)?;
            writeln!(out, "X_ACCESS_TOKEN_SECRET={}\n", access.secret.expose())?;
        }
    }
    writeln!(out, "Nothing publishes while DRY_RUN is true.")?;
    Ok(())
}

/// The command, from the environment: only the app's consumer pair is
/// needed, not the rest of the configuration.
pub async fn execute(args: &AuthXArgs) -> anyhow::Result<()> {
    let key = required("X_API_KEY")?;
    let secret = required("X_API_SECRET")?;
    let flow = ThreeLegged::new(&key, Secret::new(secret));
    let stdin = std::io::stdin();
    run(
        &flow,
        &mut stdin.lock(),
        &mut std::io::stdout(),
        args.env_file.as_deref(),
    )
    .await
}

fn required(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} is not set"))
}

/// Rewrites `key` in an env file's text in place, leaving comments and
/// order alone, or appends it.
pub fn write_env_value(contents: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let line = format!("{key}={value}");
    let mut found = false;
    let mut lines: Vec<String> = contents
        .lines()
        .map(|existing| {
            if existing.starts_with(&prefix) {
                found = true;
                line.clone()
            } else {
                existing.to_owned()
            }
        })
        .collect();
    if !found {
        lines.push(line);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Writes `contents` readable by the owner only.
fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "auth_x_tests.rs"]
mod tests;
