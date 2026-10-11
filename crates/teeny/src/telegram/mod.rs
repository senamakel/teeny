//! Telegram, on one of two transports (`TELEGRAM_TRANSPORT`):
//!
//! - **`teeny`, the default**: [`poller`], Teeny's own Bot API long poll,
//!   which hands every message to `teeny_jobs::chat`. The turn there runs
//!   `teeny-chat` as a public turn and keeps Teeny's own checks in the loop:
//!   the content filter on the way in and on the way out, the transcript and
//!   its token budget, the house style, `TELEGRAM_DRY_RUN`.
//! - **`openhuman`**: OpenHuman's Telegram channel bound to `teeny-chat`
//!   (`runtime.channels().telegram(..)`). The agent's posture still holds
//!   (its named reads, the host-only policy, the read-only public tier:
//!   nothing that writes), but
//!   the channel sends the agent's text as it is: **it bypasses Teeny's
//!   outbound content filter, the house style and the transcript budget**,
//!   keeps its own per-chat history, and ignores `TELEGRAM_DRY_RUN`. That is
//!   why it is not the default. It is compiled only with
//!   `--features openhuman-telegram` (it pulls in OpenHuman's channels).
//!
//! Either way Telegram rides with the ingest role: Telegram allows one
//! poller per token, and that role runs in exactly one place.

pub mod api;
pub mod inbox;
pub mod poller;

use std::sync::Arc;
use std::time::Duration;

use teeny_core::Config;
use teeny_core::config::TelegramTransport;

use crate::app::App;
use crate::schedule::runs_ingest;
use api::TelegramApi;
use poller::{JobsHandler, Poller, PollerOptions};

/// Which transport this process runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selected {
    /// None, and why.
    Off(String),
    Teeny,
    OpenHuman,
}

pub fn select(config: &Config) -> Selected {
    if config.telegram.bot_token.is_none() {
        return Selected::Off("no TELEGRAM_BOT_TOKEN".into());
    }
    if !runs_ingest(config.server.service_role) {
        return Selected::Off(format!(
            "SERVICE_ROLE={}: Telegram rides with the ingest role",
            config.server.service_role
        ));
    }
    match config.telegram.transport {
        TelegramTransport::Teeny => Selected::Teeny,
        TelegramTransport::OpenHuman => Selected::OpenHuman,
    }
}

/// A running transport.
pub enum Transport {
    Teeny(poller::Running),
    #[cfg(feature = "openhuman-telegram")]
    OpenHuman(openhuman_embed::ChannelListener),
}

impl Transport {
    pub async fn stop(self) {
        match self {
            Self::Teeny(running) => running.stop().await,
            #[cfg(feature = "openhuman-telegram")]
            Self::OpenHuman(listener) => listener.stop(),
        }
    }
}

/// Starts the selected transport; `None` when Telegram is off here. The
/// bot's own handle comes from the token, and without it a group line
/// cannot be told apart, so a failing `getMe` stops the boot.
pub async fn start(app: &App, api: Option<Arc<TelegramApi>>) -> anyhow::Result<Option<Transport>> {
    let config = &app.config;
    match select(config) {
        Selected::Off(why) => {
            tracing::info!(why, "telegram: off");
            Ok(None)
        }
        Selected::Teeny => {
            let api = api.ok_or_else(|| anyhow::anyhow!("no Telegram client was built"))?;
            let bot = api.get_me().await?;
            tracing::info!(handle = %format!("@{}", bot.username), dry_run = config.telegram.dry_run, "telegram: Teeny's own transport");
            let options = PollerOptions {
                bot,
                debounce: Duration::from_millis(config.telegram.debounce_ms),
                poll_timeout_s: config.telegram.poll_timeout_s,
                dry_run: config.telegram.dry_run,
            };
            let handler = Arc::new(JobsHandler::new(Arc::clone(&app.ctx)));
            Ok(Some(Transport::Teeny(
                Poller::new(api, handler, options).start(),
            )))
        }
        Selected::OpenHuman => start_openhuman(app).map(Some),
    }
}

#[cfg(feature = "openhuman-telegram")]
fn start_openhuman(app: &App) -> anyhow::Result<Transport> {
    use openhuman_embed::TelegramChannelSpec;
    use teeny_agent::posture::Belt;
    use teeny_agent::specs::agent_id;

    let token = app
        .config
        .telegram
        .bot_token
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no TELEGRAM_BOT_TOKEN"))?;
    tracing::warn!(
        "telegram: OpenHuman's channel on teeny-chat; Teeny's outbound content filter, house style and transcript budget do not run on this transport"
    );
    let listener = app.runtime.channels().telegram(
        TelegramChannelSpec::new(token.expose(), agent_id(Belt::Chat))
            .allow_everyone()
            .mention_only(true),
    )?;
    Ok(Transport::OpenHuman(listener))
}

#[cfg(not(feature = "openhuman-telegram"))]
fn start_openhuman(_app: &App) -> anyhow::Result<Transport> {
    anyhow::bail!(
        "TELEGRAM_TRANSPORT=openhuman needs a build with `--features openhuman-telegram`; \
         this one carries Teeny's own transport only (TELEGRAM_TRANSPORT=teeny)"
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
