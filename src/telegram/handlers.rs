use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MediaKind, MessageKind, ReplyMarkup};
use tracing::{info, warn};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::config::{Config, TradingMode};
use crate::market_data::collector::SharedMarketData;
use crate::strategy::signals::SignalSource;
use crate::trading::engine::{SharedDb, SharedSessions};
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
    pub db: SharedDb,
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
            sessions.insert(user_id, portfolio.clone());
        }

        // Persist to DynamoDB
        if let Some(ref db) = deps.db {
            if let Err(e) = db.save_session(&portfolio).await {
                warn!("Failed to persist new session for user {user_id}: {e}");
            }
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

/// Build extended portfolio text with config info
fn build_portfolio_text(portfolio: &Portfolio, config: &Config) -> String {
    let total = portfolio.total_value();
    let total_pnl = total - portfolio.initial_balance;
    let total_pnl_pct = if portfolio.initial_balance > Decimal::ZERO {
        total_pnl / portfolio.initial_balance * dec!(100)
    } else {
        Decimal::ZERO
    };

    let pnl_sign = if total_pnl >= Decimal::ZERO { "+" } else { "-" };

    // P&L by strategy
    let arb_pnl: Decimal = portfolio.trade_history.iter()
        .filter(|t| t.source == SignalSource::Arbitrage)
        .map(|t| t.realized_pnl).sum();
    let mom_pnl: Decimal = portfolio.trade_history.iter()
        .filter(|t| t.source == SignalSource::Momentum)
        .map(|t| t.realized_pnl).sum();
    let mr_pnl: Decimal = portfolio.trade_history.iter()
        .filter(|t| t.source == SignalSource::MeanReversion)
        .map(|t| t.realized_pnl).sum();
    let tail_pnl: Decimal = portfolio.trade_history.iter()
        .filter(|t| t.source == SignalSource::TailRisk)
        .map(|t| t.realized_pnl).sum();

    // Trade counts by close reason
    let tp_count = portfolio.trade_history.iter()
        .filter(|t| t.close_reason == "take_profit").count();
    let sl_count = portfolio.trade_history.iter()
        .filter(|t| t.close_reason == "stop_loss").count();

    // Win rate
    let winning = portfolio.trade_history.iter()
        .filter(|t| t.realized_pnl > Decimal::ZERO).count();
    let total_trades = portfolio.trade_history.len();
    let win_rate = if total_trades > 0 {
        format!("{:.0}%", winning as f64 / total_trades as f64 * 100.0)
    } else {
        "N/A".to_string()
    };

    // Mode
    let mode_str = match config.trading_mode {
        TradingMode::Paper => "Paper Trading",
        TradingMode::Live => "LIVE Trading",
    };

    // Polymarket account
    let wallet_str = match &config.wallet_private_key {
        Some(_) => "Connected",
        None => "Not connected",
    };

    format!(
        "Portfolio\n\
         ─────────────────\n\
         Mode: {mode_str}\n\
         Polymarket: {wallet_str}\n\
         \n\
         Balance: ${:.2}\n\
         Open positions: {}\n\
         Total value: ${:.2}\n\
         \n\
         P&L: {pnl_sign}${:.2} ({pnl_sign}{:.1}%)\n\
         Unrealized: ${:.2}\n\
         \n\
         Strategy: {}% Arb / {}% Mom / {}% MR / {}% Tail\n\
         Arb P&L: ${:.2}\n\
         Momentum P&L: ${:.2}\n\
         Mean Rev P&L: ${:.2}\n\
         Tail Risk P&L: ${:.2}\n\
         \n\
         Trades: {} (TP: {} / SL: {})\n\
         Win rate: {win_rate}\n\
         Risk: TP {}% / SL {}%\n\
         Max bet: ${} | Max pos: {}",
        portfolio.balance,
        portfolio.num_open_positions(),
        total,
        total_pnl.abs(),
        total_pnl_pct.abs(),
        portfolio.total_unrealized_pnl(),
        config.arb_budget_pct * dec!(100),
        config.momentum_budget_pct * dec!(100),
        config.mean_reversion_budget_pct * dec!(100),
        config.tail_risk_budget_pct * dec!(100),
        arb_pnl,
        mom_pnl,
        mr_pnl,
        tail_pnl,
        total_trades,
        tp_count,
        sl_count,
        config.take_profit_pct,
        config.stop_loss_pct,
        config.max_bet_usd,
        config.max_open_positions,
    )
}

/// Handle /status command — show portfolio summary
pub async fn handle_status(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let sessions = deps.sessions.read().await;
    match sessions.get(&user_id) {
        Some(portfolio) => {
            let text = build_portfolio_text(portfolio, &deps.config);
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
pub async fn handle_strategy(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let c = &deps.config;
    bot.send_message(
        msg.chat.id,
        format!(
            "Strategy Configuration\n\
             ─────────────────────\n\n\
             Current: Arb {}% / Mom {}% / MR {}%\n\
             MR threshold: {}% | Mom threshold: {}%\n\
             Risk: TP {}% / SL {}%\n\n\
             Choose allocation (Arb/Mom/MR):",
            c.arb_budget_pct * dec!(100),
            c.momentum_budget_pct * dec!(100),
            c.mean_reversion_budget_pct * dec!(100),
            c.mean_reversion_threshold * dec!(100),
            c.momentum_derivative_threshold * dec!(100),
            c.take_profit_pct,
            c.stop_loss_pct,
        ),
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

/// Handle /stop command — show stop + reset options
pub async fn handle_stop(bot: Bot, msg: Message, _deps: BotDeps) -> ResponseResult<()> {
    bot.send_message(
        msg.chat.id,
        "What would you like to do?",
    )
    .reply_markup(keyboards::stop_menu())
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
            match sessions.get(&user_id) {
                Some(portfolio) => {
                    let text = build_portfolio_text(portfolio, &deps.config);
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu())
                        .await?;
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.")
                        .await?;
                }
            }
        }
        "cmd:markets" => {
            let md = deps.market_data.read().await;
            let markets = &md.tracked_markets;

            if markets.is_empty() {
                bot.send_message(chat_id, "No markets currently being tracked.\nThe collector may still be initializing...")
                    .reply_markup(keyboards::main_menu())
                    .await?;
            } else {
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
                bot.send_message(chat_id, text)
                    .reply_markup(keyboards::main_menu())
                    .await?;
            }
        }
        "cmd:trades" => {
            let sessions = deps.sessions.read().await;
            match sessions.get(&user_id) {
                Some(portfolio) => {
                    let mut text = String::from("Recent Trades\n─────────────\n");

                    if portfolio.trade_history.is_empty() && portfolio.open_positions.is_empty() {
                        text.push_str("\nNo trades yet. The bot is scanning for opportunities...");
                    }

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

                    let recent: Vec<_> = portfolio.trade_history.iter().rev().take(10).collect();
                    if !recent.is_empty() {
                        text.push_str("\nRecent Closed:\n");
                        for trade in recent {
                            let pnl_sign = if trade.realized_pnl >= rust_decimal::Decimal::ZERO { "+" } else { "" };
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

                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu())
                        .await?;
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.")
                        .await?;
                }
            }
        }
        "cmd:strategy" => {
            let c = &deps.config;
            bot.send_message(
                chat_id,
                format!(
                    "Strategy Configuration\n\
                     ─────────────────────\n\n\
                     Current: Arb {}% / Mom {}% / MR {}%\n\
                     MR threshold: {}% | Mom threshold: {}%\n\
                     Risk: TP {}% / SL {}%\n\n\
                     Choose allocation (Arb/Mom/MR):",
                    c.arb_budget_pct * dec!(100),
                    c.momentum_budget_pct * dec!(100),
                    c.mean_reversion_budget_pct * dec!(100),
                    c.mean_reversion_threshold * dec!(100),
                    c.momentum_derivative_threshold * dec!(100),
                    c.take_profit_pct,
                    c.stop_loss_pct,
                ),
            )
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
            bot.send_message(chat_id, "What would you like to do?")
                .reply_markup(keyboards::stop_menu())
                .await?;
        }
        "confirm:stop" => {
            bot.send_message(chat_id, "Trading paused. Use /start to resume.")
                .reply_markup(keyboards::main_menu())
                .await?;
        }
        "confirm:reset" => {
            let initial_balance = deps.config.paper_balance;
            let new_portfolio = Portfolio::new(user_id, initial_balance);
            {
                let mut sessions = deps.sessions.write().await;
                sessions.insert(user_id, new_portfolio.clone());
            }
            // Persist reset to DB
            if let Some(ref db) = deps.db {
                if let Err(e) = db.save_session(&new_portfolio).await {
                    warn!("Failed to persist reset session: {e}");
                }
            }
            info!("User {} reset paper session to ${}", user_id, initial_balance);
            bot.send_message(
                chat_id,
                format!(
                    "Paper session reset!\n\n\
                     Balance: ${:.2}\n\
                     Open positions: 0\n\
                     Trade history: cleared\n\n\
                     Bot will resume trading automatically.",
                    initial_balance
                ),
            )
            .reply_markup(keyboards::main_menu())
            .await?;
        }
        "ask:reset" => {
            bot.send_message(
                chat_id,
                "This will erase all positions and trade history, and reset your balance to the initial amount.\n\n\
                 Are you sure?",
            )
            .reply_markup(keyboards::confirm_reset_keyboard())
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
