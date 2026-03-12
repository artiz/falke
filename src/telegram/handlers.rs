use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MediaKind, MessageKind, ReplyMarkup};
use tracing::{info, warn};

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;
use crate::trading::engine::SharedSessions;
use crate::trading::portfolio::Portfolio;

use super::auth::PhoneAuth;
use super::keyboards;

/// Shared dependencies for all handlers
#[derive(Clone)]
pub struct BotDeps {
    pub config: Arc<Config>,
    pub phone_auth: Arc<PhoneAuth>,
    pub sessions: SharedSessions,
    pub market_data: SharedMarketData,
}

/// Handle the /start command — initiate registration
pub async fn handle_start(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    // Check if already registered
    {
        let sessions = deps.sessions.read().await;
        if sessions.contains_key(&user_id) {
            bot.send_message(
                msg.chat.id,
                "You're already registered! Use the menu below.",
            )
            .reply_markup(keyboards::main_menu())
            .await?;
            return Ok(());
        }
    }

    bot.send_message(
        msg.chat.id,
        "Welcome to Falke Trading Bot!\n\n\
         To register, please share your phone number.\n\
         Only pre-approved numbers can access the bot.",
    )
    .reply_markup(keyboards::phone_request_keyboard())
    .await?;

    Ok(())
}

/// Handle contact sharing (phone number verification)
pub async fn handle_contact(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let contact = match &msg.kind {
        MessageKind::Common(common) => match &common.media_kind {
            MediaKind::Contact(c) => &c.contact,
            _ => return Ok(()),
        },
        _ => return Ok(()),
    };

    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let phone = &contact.phone_number;

    if deps.phone_auth.is_authorized(phone) {
        // Register the user
        let initial_balance = deps.config.paper_balance;
        let portfolio = Portfolio::new(user_id, initial_balance);

        {
            let mut sessions = deps.sessions.write().await;
            sessions.insert(user_id, portfolio);
        }

        info!("User registered: telegram_id={user_id}, phone={phone}");

        bot.send_message(
            msg.chat.id,
            format!(
                "Registration successful!\n\n\
                 Mode: Paper Trading\n\
                 Balance: ${:.2}\n\
                 Strategy: 50% Arbitrage / 50% Momentum\n\n\
                 The bot is now monitoring markets and will trade automatically.\n\
                 Use the menu below to check your portfolio.",
                initial_balance
            ),
        )
        .reply_markup(ReplyMarkup::InlineKeyboard(keyboards::main_menu()))
        .await?;
    } else {
        warn!("Unauthorized registration attempt: phone={phone}");

        bot.send_message(
            msg.chat.id,
            "Sorry, your phone number is not authorized.\n\
             Contact the bot administrator for access.",
        )
        .reply_markup(keyboards::remove_keyboard())
        .await?;
    }

    Ok(())
}

/// Handle /status command — show portfolio summary
pub async fn handle_status(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let sessions = deps.sessions.read().await;
    match sessions.get(&user_id) {
        Some(portfolio) => {
            bot.send_message(msg.chat.id, portfolio.summary())
                .reply_markup(keyboards::main_menu())
                .await?;
        }
        None => {
            bot.send_message(msg.chat.id, "You're not registered. Use /start to begin.")
                .await?;
        }
    }

    Ok(())
}

/// Handle /markets command — show tracked markets
pub async fn handle_markets(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let data = deps.market_data.read().await;
    let markets = &data.tracked_markets;

    if markets.is_empty() {
        bot.send_message(msg.chat.id, "No markets currently being tracked.")
            .await?;
        return Ok(());
    }

    let mut text = format!("Tracked Markets ({})\n─────────────────\n", markets.len());

    for (i, market) in markets.iter().take(20).enumerate() {
        let prices: Vec<String> = market
            .outcomes
            .iter()
            .map(|o| format!("{}: {:.1}c", o.name, o.price * rust_decimal_macros::dec!(100)))
            .collect();

        let price_sum: rust_decimal::Decimal = market.outcomes.iter().map(|o| o.price).sum();

        text.push_str(&format!(
            "\n{}. {}\n   {} | Sum: {:.2}\n",
            i + 1,
            truncate(&market.question, 50),
            prices.join(" / "),
            price_sum,
        ));
    }

    if markets.len() > 20 {
        text.push_str(&format!("\n... and {} more", markets.len() - 20));
    }

    bot.send_message(msg.chat.id, text)
        .reply_markup(keyboards::main_menu())
        .await?;

    Ok(())
}

/// Handle /trades command — show recent trades
pub async fn handle_trades(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let sessions = deps.sessions.read().await;
    match sessions.get(&user_id) {
        Some(portfolio) => {
            let mut text = String::from("Recent Trades\n─────────────\n");

            if portfolio.trade_history.is_empty() && portfolio.open_positions.is_empty() {
                text.push_str("\nNo trades yet. The bot is scanning for opportunities...");
            }

            // Open positions
            if !portfolio.open_positions.is_empty() {
                text.push_str("\nOpen Positions:\n");
                for pos in portfolio.open_positions.values().take(10) {
                    text.push_str(&format!(
                        "  {} {} @ {:.2}c | P&L: ${:.2}\n",
                        pos.side,
                        truncate(&pos.outcome_name, 20),
                        pos.entry_price * rust_decimal_macros::dec!(100),
                        pos.unrealized_pnl(),
                    ));
                }
            }

            // Recent closed trades
            let recent: Vec<_> = portfolio.trade_history.iter().rev().take(10).collect();
            if !recent.is_empty() {
                text.push_str("\nRecent Closed:\n");
                for trade in recent {
                    let pnl_sign = if trade.realized_pnl >= rust_decimal::Decimal::ZERO {
                        "+"
                    } else {
                        ""
                    };
                    text.push_str(&format!(
                        "  {} {} | {}{:.2} ({:.1}%)\n",
                        trade.side,
                        truncate(&trade.outcome_name, 20),
                        pnl_sign,
                        trade.realized_pnl,
                        trade.realized_pnl_pct,
                    ));
                }
            }

            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu())
                .await?;
        }
        None => {
            bot.send_message(msg.chat.id, "You're not registered. Use /start to begin.")
                .await?;
        }
    }

    Ok(())
}

/// Handle /strategy command
pub async fn handle_strategy(bot: Bot, msg: Message, _deps: BotDeps) -> ResponseResult<()> {
    bot.send_message(
        msg.chat.id,
        "Strategy Configuration\n─────────────────────\n\n\
         Choose your allocation between Arbitrage and Momentum strategies:",
    )
    .reply_markup(keyboards::strategy_keyboard())
    .await?;

    Ok(())
}

/// Handle /mode command
pub async fn handle_mode(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let sessions = deps.sessions.read().await;
    let current_mode = match sessions.get(&user_id) {
        Some(_portfolio) => &deps.config.trading_mode,
        None => {
            bot.send_message(msg.chat.id, "You're not registered. Use /start to begin.")
                .await?;
            return Ok(());
        }
    };

    bot.send_message(
        msg.chat.id,
        format!(
            "Trading Mode\n────────────\n\nCurrent: {:?}\n\nSelect mode:",
            current_mode
        ),
    )
    .reply_markup(keyboards::mode_keyboard())
    .await?;

    Ok(())
}

/// Handle /stop command
pub async fn handle_stop(bot: Bot, msg: Message, _deps: BotDeps) -> ResponseResult<()> {
    bot.send_message(
        msg.chat.id,
        "Are you sure you want to pause all trading?",
    )
    .reply_markup(keyboards::confirm_keyboard("stop"))
    .await?;

    Ok(())
}

/// Handle callback queries from inline keyboards
pub async fn handle_callback(bot: Bot, q: CallbackQuery, deps: BotDeps) -> ResponseResult<()> {
    let data = match q.data.as_ref() {
        Some(d) => d.clone(),
        None => return Ok(()),
    };

    let chat_id = match q.message.as_ref() {
        Some(msg) => msg.chat().id,
        None => return Ok(()),
    };

    let user_id = q.from.id.0 as i64;

    // Acknowledge the callback
    bot.answer_callback_query(&q.id).await?;

    match data.as_str() {
        "cmd:status" => {
            let sessions = deps.sessions.read().await;
            if let Some(portfolio) = sessions.get(&user_id) {
                bot.send_message(chat_id, portfolio.summary())
                    .reply_markup(keyboards::main_menu())
                    .await?;
            }
        }
        "cmd:markets" => {
            let data = deps.market_data.read().await;
            let count = data.tracked_markets.len();
            let stats = data.price_store.stats();
            bot.send_message(
                chat_id,
                format!(
                    "Markets: {count} tracked\nData points: {}\nMemory: ~{} KB",
                    stats.total_data_points,
                    stats.approx_memory_bytes / 1024
                ),
            )
            .reply_markup(keyboards::main_menu())
            .await?;
        }
        "cmd:trades" => {
            let sessions = deps.sessions.read().await;
            if let Some(portfolio) = sessions.get(&user_id) {
                let open = portfolio.num_open_positions();
                let closed = portfolio.trade_history.len();
                bot.send_message(
                    chat_id,
                    format!("Open: {open} | Closed: {closed}"),
                )
                .reply_markup(keyboards::main_menu())
                .await?;
            }
        }
        "cmd:strategy" => {
            bot.send_message(chat_id, "Select strategy allocation:")
                .reply_markup(keyboards::strategy_keyboard())
                .await?;
        }
        "cmd:mode" => {
            bot.send_message(chat_id, "Select trading mode:")
                .reply_markup(keyboards::mode_keyboard())
                .await?;
        }
        "cmd:menu" => {
            bot.send_message(chat_id, "Main Menu")
                .reply_markup(keyboards::main_menu())
                .await?;
        }
        "cmd:stop" => {
            bot.send_message(chat_id, "Confirm stop trading?")
                .reply_markup(keyboards::confirm_keyboard("stop"))
                .await?;
        }
        "confirm:stop" => {
            bot.send_message(chat_id, "Trading paused. Use /start to resume.")
                .await?;
        }
        _ => {
            if data.starts_with("strategy:") || data.starts_with("mode:") {
                bot.send_message(chat_id, format!("Setting updated: {data}"))
                    .reply_markup(keyboards::main_menu())
                    .await?;
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() > max_len {
        &s[..max_len]
    } else {
        s
    }
}
