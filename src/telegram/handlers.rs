use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MediaKind, MessageKind, ReplyMarkup};
use tracing::{info, warn};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::config::{Config, SharedConfig, TradingMode};
use crate::market_data::collector::SharedMarketData;
use crate::trading::engine::{SharedDb, SharedSessions};
use crate::trading::portfolio::Portfolio;
use crate::trading::testing::SharedTestSessions;

use super::auth::PhoneAuth;
use super::keyboards;

/// Shared dependencies for all handlers
#[derive(Clone)]
pub struct BotDeps {
    pub config: SharedConfig,
    pub phone_auth: Arc<PhoneAuth>,
    pub sessions: SharedSessions,
    pub market_data: SharedMarketData,
    pub db: SharedDb,
    pub test_sessions: Option<SharedTestSessions>,
}

/// Read paused + testing_mode from config in one lock
async fn menu_state(deps: &BotDeps) -> (bool, bool) {
    let cfg = deps.config.read().await;
    (cfg.trading_paused, cfg.testing_mode)
}

/// Handle the /start command — initiate registration
pub async fn handle_start(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    {
        let sessions = deps.sessions.read().await;
        if sessions.contains_key(&user_id) {
            let was_paused = deps.config.read().await.trading_paused;
            if was_paused {
                deps.config.write().await.trading_paused = false;
                info!("User {} resumed trading", user_id);
            }
            let msg_text = if was_paused {
                "Trading resumed!".to_string()
            } else {
                "You're already registered! Use the menu below.".to_string()
            };
            let (paused, testing) = menu_state(&deps).await;
            bot.send_message(msg.chat.id, msg_text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing))
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
        let initial_balance = deps.config.read().await.paper_balance;
        let portfolio = Portfolio::new(user_id, initial_balance);

        {
            let mut sessions = deps.sessions.write().await;
            sessions.insert(user_id, portfolio.clone());
        }

        if let Some(ref db) = deps.db {
            if let Err(e) = db.save_session(&portfolio).await {
                warn!("Failed to persist new session for user {user_id}: {e}");
            }
        }

        info!("User registered: telegram_id={user_id}, phone={phone}");

        let (paused, testing) = menu_state(&deps).await;
        bot.send_message(
            msg.chat.id,
            format!(
                "Registration successful!\n\n\
                 Mode: Paper Trading\n\
                 Balance: ${:.2}\n\
                 Strategy: Tail Risk\n\n\
                 The bot is now monitoring markets and will trade automatically.\n\
                 Use the menu below to check your portfolio.",
                initial_balance
            ),
        )
        .reply_markup(ReplyMarkup::InlineKeyboard(
            keyboards::main_menu_with_state(paused, testing),
        ))
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

/// Build portfolio text
fn build_portfolio_text(portfolio: &Portfolio, config: &Config) -> String {
    let total = portfolio.total_value();
    let total_pnl = total - portfolio.initial_balance;
    let total_pnl_pct = if portfolio.initial_balance > Decimal::ZERO {
        total_pnl / portfolio.initial_balance * dec!(100)
    } else {
        Decimal::ZERO
    };

    let pnl_sign = if total_pnl >= Decimal::ZERO { "+" } else { "-" };

    let tail_pnl: Decimal = portfolio.trade_history.iter().map(|t| t.realized_pnl).sum();

    let tp_count = portfolio.trade_history.iter().filter(|t| t.close_reason == "take_profit").count();
    let sl_count = portfolio.trade_history.iter().filter(|t| t.close_reason == "stop_loss").count();
    let resolved_win_count = portfolio.trade_history.iter().filter(|t| t.close_reason == "resolved_win").count();
    let resolved_loss_count = portfolio.trade_history.iter().filter(|t| t.close_reason == "resolved_loss").count();

    let winning = portfolio.trade_history.iter().filter(|t| t.realized_pnl > Decimal::ZERO).count();
    let total_trades = portfolio.trade_history.len();
    let win_rate = if total_trades > 0 {
        format!("{:.0}%", winning as f64 / total_trades as f64 * 100.0)
    } else {
        "N/A".to_string()
    };

    let mode_str = match (&config.trading_mode, config.trading_paused) {
        (_, true) => "PAUSED",
        (TradingMode::Paper, false) => "Paper Trading",
        (TradingMode::Live, false) => "LIVE Trading",
    };

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
         Tail Risk P&L: ${:.2}\n\
         \n\
         Trades: {} (TP: {} / SL: {} / Win: {} / Loss: {})\n\
         Win rate: {win_rate}\n\
         Tail TP {}% / SL {}%\n\
         Max price: {}c | Bet: ${} | Max bet: ${} | Max pos: {}\n\
         Brake: {}% loss → pause {}min",
        portfolio.balance,
        portfolio.num_open_positions(),
        total,
        total_pnl.abs(),
        total_pnl_pct.abs(),
        portfolio.total_unrealized_pnl(),
        tail_pnl,
        total_trades,
        tp_count,
        sl_count,
        resolved_win_count,
        resolved_loss_count,
        config.tail_risk_take_profit_pct,
        config.tail_risk_stop_loss_pct,
        config.tail_risk_max_price * dec!(100),
        config.tail_risk_bet_usd,
        config.max_bet_usd,
        config.max_open_positions,
        config.budget_brake_pct,
        config.budget_brake_time_sec / 60,
    )
}

/// Build test leaderboard text
async fn build_test_results_text(deps: &BotDeps) -> String {
    let ts = match &deps.test_sessions {
        Some(ts) => ts,
        None => return "Testing mode is not enabled.".to_string(),
    };

    let ts_lock = ts.read().await;
    if ts_lock.is_empty() {
        return "No test portfolios initialized.".to_string();
    }

    let mut ranked: Vec<(Decimal, usize)> = ts_lock
        .iter()
        .enumerate()
        .map(|(i, tp)| {
            let pnl_pct = if tp.portfolio.initial_balance > Decimal::ZERO {
                (tp.portfolio.total_value() - tp.portfolio.initial_balance)
                    / tp.portfolio.initial_balance
                    * dec!(100)
            } else {
                Decimal::ZERO
            };
            (pnl_pct, i)
        })
        .collect();

    ranked.sort_by(|a, b| b.0.cmp(&a.0));

    let mut text = format!(
        "Test Leaderboard ({} strategies)\n─────────────────\n",
        ts_lock.len()
    );

    for (rank, (pnl_pct, idx)) in ranked.iter().take(10).enumerate() {
        let tp = &ts_lock[*idx];
        let pnl_usd = tp.portfolio.total_value() - tp.portfolio.initial_balance;
        let sign = if *pnl_pct >= Decimal::ZERO { "+" } else { "" };
        text.push_str(&format!(
            "{}. {} | {}{:.1}% (${:.2}) | {} trades\n",
            rank + 1,
            tp.config.name,
            sign,
            pnl_pct,
            pnl_usd,
            tp.portfolio.trade_history.len(),
        ));
    }

    text
}

/// Handle /status command
pub async fn handle_status(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let sessions = deps.sessions.read().await;
    match sessions.get(&user_id) {
        Some(portfolio) => {
            let text = build_portfolio_text(portfolio, &*deps.config.read().await);
            let (paused, testing) = menu_state(&deps).await;
            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing))
                .await?;
        }
        None => {
            bot.send_message(msg.chat.id, "You're not registered. Use /start to begin.")
                .await?;
        }
    }
    Ok(())
}

/// Handle /markets command
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
            i + 1, truncate(&market.question, 50), prices.join(" / "), price_sum,
        ));
    }
    if markets.len() > 20 {
        text.push_str(&format!("\n... and {} more", markets.len() - 20));
    }

    let (paused, testing) = menu_state(&deps).await;
    bot.send_message(msg.chat.id, text)
        .reply_markup(keyboards::main_menu_with_state(paused, testing))
        .await?;
    Ok(())
}

/// Handle /trades command
pub async fn handle_trades(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
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
                        "  [TAIL] {} @ {:.2}c | P&L: ${:.2}\n",
                        truncate(&pos.outcome_name, 20),
                        pos.entry_price * rust_decimal_macros::dec!(100),
                        pos.unrealized_pnl(),
                    ));
                }
            }

            let recent: Vec<_> = portfolio.trade_history.iter().rev().take(20).collect();
            if !recent.is_empty() {
                text.push_str("\nRecent Closed:\n");
                for trade in recent {
                    let (emoji, pnl_sign) = if trade.realized_pnl >= rust_decimal::Decimal::ZERO {
                        ("\u{1f7e2}", "+")
                    } else {
                        ("\u{1f534}", "")
                    };
                    text.push_str(&format!(
                        "{} [TAIL] {} | {}{:.2} ({:.1}%)\n",
                        emoji,
                        truncate(&trade.outcome_name, 20),
                        pnl_sign,
                        trade.realized_pnl,
                        trade.realized_pnl_pct,
                    ));
                }
            }

            let (paused, testing) = menu_state(&deps).await;
            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing))
                .await?;
        }
        None => {
            bot.send_message(msg.chat.id, "You're not registered. Use /start to begin.")
                .await?;
        }
    }
    Ok(())
}

/// Handle /test command — show test leaderboard
pub async fn handle_test_results(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let text = build_test_results_text(&deps).await;
    let (paused, testing) = menu_state(&deps).await;
    bot.send_message(msg.chat.id, text)
        .reply_markup(keyboards::main_menu_with_state(paused, testing))
        .await?;
    Ok(())
}

/// Handle /stop command
pub async fn handle_stop(bot: Bot, msg: Message, _deps: BotDeps) -> ResponseResult<()> {
    bot.send_message(msg.chat.id, "What would you like to do?")
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

    bot.answer_callback_query(&q.id).await?;

    match data.as_str() {
        "cmd:status" => {
            let sessions = deps.sessions.read().await;
            match sessions.get(&user_id) {
                Some(portfolio) => {
                    let text = build_portfolio_text(portfolio, &*deps.config.read().await);
                    let (paused, testing) = menu_state(&deps).await;
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu_with_state(paused, testing))
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
            let (paused, testing) = menu_state(&deps).await;

            if markets.is_empty() {
                bot.send_message(chat_id, "No markets currently being tracked.\nThe collector may still be initializing...")
                    .reply_markup(keyboards::main_menu_with_state(paused, testing))
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
                        i + 1, truncate(&market.question, 50), prices.join(" / "), price_sum,
                    ));
                }
                if markets.len() > 20 {
                    text.push_str(&format!("\n... and {} more", markets.len() - 20));
                }
                bot.send_message(chat_id, text)
                    .reply_markup(keyboards::main_menu_with_state(paused, testing))
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
                        for pos in portfolio.open_positions.values().take(50) {
                            text.push_str(&format!(
                                "  [TAIL] {} @ {:.2}c | P&L: ${:.2}\n",
                                truncate(&pos.outcome_name, 20),
                                pos.entry_price * rust_decimal_macros::dec!(100),
                                pos.unrealized_pnl(),
                            ));
                        }
                    }

                    let recent: Vec<_> = portfolio.trade_history.iter().rev().take(20).collect();
                    if !recent.is_empty() {
                        text.push_str("\nRecent Closed:\n");
                        for trade in recent {
                            let (emoji, pnl_sign) =
                                if trade.realized_pnl >= rust_decimal::Decimal::ZERO {
                                    ("\u{1f7e2}", "+")
                                } else {
                                    ("\u{1f534}", "")
                                };
                            text.push_str(&format!(
                                "{} [TAIL] {} | {}{:.2} ({:.1}%)\n",
                                emoji,
                                truncate(&trade.outcome_name, 20),
                                pnl_sign,
                                trade.realized_pnl,
                                trade.realized_pnl_pct,
                            ));
                        }
                    }

                    let (paused, testing) = menu_state(&deps).await;
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu_with_state(paused, testing))
                        .await?;
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.")
                        .await?;
                }
            }
        }
        "cmd:test" => {
            let text = build_test_results_text(&deps).await;
            let (paused, testing) = menu_state(&deps).await;
            bot.send_message(chat_id, text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing))
                .await?;
        }
        "cmd:menu" => {
            let (paused, testing) = menu_state(&deps).await;
            bot.send_message(chat_id, "Main Menu")
                .reply_markup(keyboards::main_menu_with_state(paused, testing))
                .await?;
        }
        "cmd:stop" => {
            bot.send_message(chat_id, "What would you like to do?")
                .reply_markup(keyboards::stop_menu())
                .await?;
        }
        "confirm:stop" => {
            deps.config.write().await.trading_paused = true;
            info!("User {} paused trading", user_id);
            let testing = deps.config.read().await.testing_mode;
            bot.send_message(chat_id, "Trading paused.")
                .reply_markup(keyboards::main_menu_with_state(true, testing))
                .await?;
        }
        "confirm:resume" => {
            deps.config.write().await.trading_paused = false;
            info!("User {} resumed trading", user_id);
            let testing = deps.config.read().await.testing_mode;
            bot.send_message(chat_id, "Trading resumed!")
                .reply_markup(keyboards::main_menu_with_state(false, testing))
                .await?;
        }
        "confirm:reset" => {
            let initial_balance = deps.config.read().await.paper_balance;
            let new_portfolio = Portfolio::new(user_id, initial_balance);
            {
                let mut sessions = deps.sessions.write().await;
                sessions.insert(user_id, new_portfolio.clone());
            }
            if let Some(ref db) = deps.db {
                if let Err(e) = db.save_session(&new_portfolio).await {
                    warn!("Failed to persist reset session: {e}");
                }
            }
            info!("User {} reset paper session to ${}", user_id, initial_balance);
            let (paused, testing) = menu_state(&deps).await;
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
            .reply_markup(keyboards::main_menu_with_state(paused, testing))
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
            if data.starts_with("mode:") {
                let (paused, testing) = menu_state(&deps).await;
                bot.send_message(chat_id, format!("Setting updated: {data}"))
                    .reply_markup(keyboards::main_menu_with_state(paused, testing))
                    .await?;
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() > max_len { &s[..max_len] } else { s }
}
