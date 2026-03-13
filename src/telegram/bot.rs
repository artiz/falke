use std::sync::Arc;
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
use teloxide::dptree;
use teloxide::prelude::*;
use tracing::info;

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;
use crate::trading::engine::{SharedDb, SharedSessions};

use super::auth::PhoneAuth;
use super::handlers::{self, BotDeps};

/// Start the Telegram bot with Dispatcher (handles both messages and callback queries)
pub async fn run_bot(config: Config, sessions: SharedSessions, market_data: SharedMarketData, db: SharedDb, bot: Bot) {
    info!("Starting Telegram bot...");

    let phone_auth = PhoneAuth::new(config.allowed_phones.clone());

    let deps = BotDeps {
        config: Arc::new(config),
        phone_auth: Arc::new(phone_auth),
        sessions,
        market_data,
        db,
    };

    let handler = dptree::entry()
        // Handle messages (commands + contact sharing)
        .branch(
            Update::filter_message().endpoint(move |bot: Bot, msg: Message, deps: BotDeps| async move {
                if has_contact(&msg) {
                    return handlers::handle_contact(bot, msg, deps).await;
                }

                if let Some(text) = msg.text() {
                    match text {
                        "/start" => return handlers::handle_start(bot, msg, deps).await,
                        "/status" => return handlers::handle_status(bot, msg, deps).await,
                        "/markets" => return handlers::handle_markets(bot, msg, deps).await,
                        "/trades" => return handlers::handle_trades(bot, msg, deps).await,
                        "/strategy" => return handlers::handle_strategy(bot, msg, deps).await,
                        "/mode" => return handlers::handle_mode(bot, msg, deps).await,
                        "/stop" => return handlers::handle_stop(bot, msg, deps).await,
                        _ => {
                            bot.send_message(
                                msg.chat.id,
                                "Unknown command. Use /start to see available options.",
                            )
                            .await?;
                        }
                    }
                }

                Ok(())
            }),
        )
        // Handle callback queries (inline keyboard button presses)
        .branch(
            Update::filter_callback_query()
                .endpoint(handlers::handle_callback),
        );

    info!("Telegram bot is running. Waiting for messages...");

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![deps])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

fn has_contact(msg: &Message) -> bool {
    matches!(
        &msg.kind,
        teloxide::types::MessageKind::Common(common)
            if matches!(&common.media_kind, teloxide::types::MediaKind::Contact(_))
    )
}
