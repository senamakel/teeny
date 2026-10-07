//! `teeny webhook`: reads and moves where X delivers the account's
//! activity.
//!
//! X holds the delivery URL in its own configuration, not in this repo, so
//! moving the service to a new hostname is only half a move until this runs.
//!
//! - The app may hold exactly one webhook, so a move is a delete followed by
//!   a create. Between the two only the mention poll finds mentions.
//! - X sends the CRC challenge during the create, so a URL that is not
//!   serving is refused there; if it is, the previous URL is put back
//!   before the error is raised.
//! - Delivery is subscribed through the Activity API, one subscription per
//!   event type naming the webhook and the account.
//!
//! Every write previews without `--confirm`.

use std::io::Write;

use anyhow::{Context, bail};
use teeny_core::Config;
pub use teeny_x::webhook_admin::EVENTS;
use teeny_x::XApi;
use teeny_x::webhook_admin::{Subscription, Webhook};

/// `teeny webhook <list|register|subscribe|unsubscribe|delete>`
#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum WebhookCommand {
    /// Where X delivers today: each webhook and its subscriptions.
    List,
    /// Move delivery to URL: retire the old webhook, register this one and
    /// subscribe the account to every event.
    Register {
        /// The service's public `/webhooks/x` URL.
        url: String,
        /// Do it; without it, only the plan is shown.
        #[arg(long)]
        confirm: bool,
    },
    /// Subscribe the account to every event on a webhook (the registered
    /// one when none is named). Existing subscriptions are kept.
    Subscribe {
        webhook_id: Option<String>,
        #[arg(long)]
        confirm: bool,
    },
    /// Remove every subscription bound to a webhook.
    Unsubscribe {
        webhook_id: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Unsubscribe and delete a webhook.
    Delete {
        webhook_id: String,
        #[arg(long)]
        confirm: bool,
    },
}

/// `teeny webhook <command>`
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct WebhookArgs {
    #[command(subcommand)]
    pub command: WebhookCommand,
}

/// Runs `command` for the account `@account`.
pub async fn run(
    x: &XApi,
    account: &str,
    command: &WebhookCommand,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    match command {
        WebhookCommand::List => {
            writeln!(out, "Account: @{account}")?;
            describe(x, &x.list_webhooks().await?, out).await?;
            writeln!(
                out,
                "\nMove it with: teeny webhook register <url> --confirm"
            )?;
            Ok(())
        }
        WebhookCommand::Register { url, confirm } => {
            register(x, account, url, *confirm, out).await
        }
        WebhookCommand::Subscribe {
            webhook_id,
            confirm,
        } => {
            let webhook_id = match webhook_id {
                Some(id) => id.clone(),
                None => registered(x).await?,
            };
            let user_id = x.user_id().await?.to_string();
            if *confirm {
                subscribe(x, &webhook_id, &user_id, out).await
            } else {
                let have = subscribed_to(x, &webhook_id).await?;
                let missing = missing(&have, &user_id);
                if missing.is_empty() {
                    writeln!(out, "{webhook_id} already has every subscription.")?;
                }
                for event in missing {
                    writeln!(out, "Would subscribe {webhook_id} to {event}")?;
                }
                writeln!(out, "\nRe-run with --confirm.")?;
                Ok(())
            }
        }
        WebhookCommand::Unsubscribe {
            webhook_id,
            confirm,
        } => {
            if *confirm {
                return unsubscribe(x, webhook_id, out).await;
            }
            for entry in subscribed_to(x, webhook_id).await? {
                writeln!(out, "Would unsubscribe {}", entry.describe())?;
            }
            writeln!(out, "\nRe-run with --confirm.")?;
            Ok(())
        }
        WebhookCommand::Delete {
            webhook_id,
            confirm,
        } => {
            if !confirm {
                writeln!(
                    out,
                    "Would delete webhook {webhook_id}. Re-run with --confirm."
                )?;
                return Ok(());
            }
            unsubscribe(x, webhook_id, out).await?;
            x.delete_webhook(webhook_id).await?;
            writeln!(out, "Deleted {webhook_id}.")?;
            Ok(())
        }
    }
}

/// The command, from the environment.
pub async fn execute(args: &WebhookArgs) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let x = XApi::from_config(&config.x);
    let account = config.x.bot_username.trim_start_matches('@').to_owned();
    run(&x, &account, &args.command, &mut std::io::stdout()).await
}

async fn register(
    x: &XApi,
    account: &str,
    target: &str,
    confirm: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let existing = x.list_webhooks().await?;
    let already = existing
        .iter()
        .find(|hook| hook.url.as_deref() == Some(target));
    let stale: Vec<&Webhook> = existing
        .iter()
        .filter(|hook| hook.url.as_deref() != Some(target))
        .collect();

    if !confirm {
        writeln!(out, "Account: @{account}")?;
        writeln!(out, "\nWould register: {target}")?;
        if already.is_some() {
            writeln!(out, "  (already registered; it would only be subscribed)")?;
        }
        for hook in &stale {
            writeln!(
                out,
                "Would first retire: {} ({})",
                hook.url.as_deref().unwrap_or("?"),
                hook.id.as_deref().unwrap_or("?")
            )?;
        }
        if !stale.is_empty() {
            writeln!(
                out,
                "\nThe app holds one webhook, so the old one goes before the new one arrives."
            )?;
            writeln!(out, "Between the two, only the mention poll finds mentions.")?;
        }
        writeln!(out, "\nRe-run with --confirm.")?;
        return Ok(());
    }

    let webhook_id = match already.and_then(|hook| hook.id.clone()) {
        Some(id) => {
            writeln!(out, "{target} is already registered as {id}.")?;
            id
        }
        None => {
            for hook in &stale {
                let Some(id) = &hook.id else { continue };
                writeln!(
                    out,
                    "Retiring {} ({id}) ...",
                    hook.url.as_deref().unwrap_or("?")
                )?;
                unsubscribe(x, id, out).await?;
                x.delete_webhook(id).await?;
            }
            writeln!(out, "Registering {target} ...")?;
            match x.create_webhook(target).await {
                Ok(id) => {
                    writeln!(out, "  id {id}")?;
                    id
                }
                Err(error) => {
                    if let Some(previous) = stale.first().and_then(|hook| hook.url.as_deref()) {
                        writeln!(out, "  refused; putting {previous} back ...")?;
                        if x.create_webhook(previous).await.is_err() {
                            writeln!(
                                out,
                                "  COULD NOT RESTORE {previous}. The account has no webhook and only the\n  \
                                 mention poll finds mentions. Re-run: teeny webhook register {previous} --confirm"
                            )?;
                        }
                    }
                    return Err(error.into());
                }
            }
        }
    };

    writeln!(out, "Subscribing the account ...")?;
    let user_id = x.user_id().await?.to_string();
    subscribe(x, &webhook_id, &user_id, out).await?;
    writeln!(out, "\nDone. X now delivers to:")?;
    describe(x, &x.list_webhooks().await?, out).await
}

/// The one registered webhook's id.
async fn registered(x: &XApi) -> anyhow::Result<String> {
    x.list_webhooks()
        .await?
        .into_iter()
        .find_map(|hook| hook.id)
        .context("no webhook is registered; register one first")
}

async fn subscribed_to(x: &XApi, webhook_id: &str) -> anyhow::Result<Vec<Subscription>> {
    Ok(x
        .list_subscriptions()
        .await?
        .into_iter()
        .filter(|entry| entry.webhook_id.as_deref() == Some(webhook_id))
        .collect())
}

/// The events `user_id` is not yet subscribed to among `have`.
fn missing(have: &[Subscription], user_id: &str) -> Vec<&'static str> {
    EVENTS
        .iter()
        .copied()
        .filter(|event| {
            !have.iter().any(|entry| {
                entry.event_type.as_deref() == Some(*event) && entry.user_id() == Some(user_id)
            })
        })
        .collect()
}

/// Creates the missing subscriptions, then checks X now has them all.
async fn subscribe(
    x: &XApi,
    webhook_id: &str,
    user_id: &str,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let have = subscribed_to(x, webhook_id).await?;
    for event in missing(&have, user_id) {
        x.create_subscription(webhook_id, event, user_id).await?;
        writeln!(out, "  subscribed {event}")?;
    }
    let bound = subscribed_to(x, webhook_id).await?;
    let still = missing(&bound, user_id);
    if !still.is_empty() {
        bail!(
            "the webhook is missing subscriptions for {}, so X will not deliver those. \
             Subscribe with an access token belonging to the bot account.",
            still.join(", ")
        );
    }
    let names: Vec<String> = bound.iter().map(Subscription::describe).collect();
    writeln!(out, "  subscribed: {}", names.join(", "))?;
    Ok(())
}

/// Removes the subscriptions left pointing at a webhook.
async fn unsubscribe(x: &XApi, webhook_id: &str, out: &mut dyn Write) -> anyhow::Result<()> {
    for entry in subscribed_to(x, webhook_id).await? {
        let Some(id) = &entry.subscription_id else {
            continue;
        };
        x.delete_subscription(id).await?;
        writeln!(out, "  unsubscribed {}", entry.describe())?;
    }
    Ok(())
}

async fn describe(x: &XApi, hooks: &[Webhook], out: &mut dyn Write) -> anyhow::Result<()> {
    if hooks.is_empty() {
        writeln!(
            out,
            "\n  Nothing is registered. X delivers nothing; only the mention poll finds mentions."
        )?;
        return Ok(());
    }
    for hook in hooks {
        writeln!(out, "\n  {}", hook.url.as_deref().unwrap_or("?"))?;
        writeln!(
            out,
            "    id {}   valid {}   created {}",
            hook.id.as_deref().unwrap_or("?"),
            hook.valid.map_or("?".to_owned(), |valid| valid.to_string()),
            hook.created_at.as_deref().unwrap_or("unknown")
        )?;
        let Some(id) = &hook.id else { continue };
        match subscribed_to(x, id).await {
            Ok(bound) if bound.is_empty() => writeln!(out, "    subscriptions: none")?,
            Ok(bound) => {
                let names: Vec<String> = bound.iter().map(Subscription::describe).collect();
                writeln!(out, "    subscriptions: {}", names.join(", "))?;
            }
            Err(error) => writeln!(out, "    subscriptions unreadable: {error}")?,
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "webhook_tests.rs"]
mod tests;
