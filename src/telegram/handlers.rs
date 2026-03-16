use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MediaKind, MessageKind, ReplyMarkup};
use tracing::{info, warn};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::config::{Config, SharedConfig, TradingMode};
use crate::market_data::collector::SharedMarketData;
use crate::trading::engine::{SharedDb, SharedSessions};
use crate::trading::executor::LiveExecutor;
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
    pub live_executor: Option<Arc<LiveExecutor>>,
}

/// Read paused + testing_mode + is_live from config in one lock
async fn menu_state(deps: &BotDeps) -> (bool, bool, bool) {
    let cfg = deps.config.read().await;
    (cfg.trading_paused, cfg.testing_mode, cfg.trading_mode == TradingMode::Live)
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
            let (paused, testing, is_live) = menu_state(&deps).await;
            bot.send_message(msg.chat.id, msg_text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
        let initial_balance = live_balance_or_paper(&deps).await;
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

        let (paused, testing, is_live) = menu_state(&deps).await;
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
            keyboards::main_menu_with_state(paused, testing, is_live),
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
    let positions_value = total - portfolio.balance;
    let cash_pct = if total > Decimal::ZERO {
        portfolio.balance / total * dec!(100)
    } else {
        Decimal::ZERO
    };
    let pos_pct = dec!(100) - cash_pct;

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

    let clob_balance_line = if config.trading_mode == TradingMode::Live {
        if let Some(clob_bal) = portfolio.live_clob_balance {
            format!("CLOB USDC: ${:.2}\n         ", clob_bal)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    format!(
        "Portfolio\n\
         ─────────────────\n\
         Mode: {mode_str}\n\
         Polymarket: {wallet_str}\n\
         {clob_balance_line}\n\
         Cash: ${:.2} ({:.1}%)\n\
         Positions: ${:.2} ({:.1}%) × {}\n\
         Total: ${:.2}\n\
         Unrealized: ${:.2}\n\
         \n\
         Trades: {} (TP: {} / SL: {} / Win: {} / Loss: {})\n\
         Win rate: {win_rate}\n\
         Tail TP {}% / SL {}%\n\
         Max price: {}c | Bet: ${} | Max bet: ${} | Max pos: {}\n\
         Brake: {}% loss → pause {}min",
        portfolio.balance,
        cash_pct,
        positions_value,
        pos_pct,
        portfolio.num_open_positions(),
        total,
        portfolio.total_unrealized_pnl(),
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
            let (paused, testing, is_live) = menu_state(&deps).await;
            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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

    let (paused, testing, is_live) = menu_state(&deps).await;
    bot.send_message(msg.chat.id, text)
        .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
        .await?;
    Ok(())
}

/// Handle /trades command
pub async fn handle_trades(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let end_dates = build_end_date_map(&deps).await;
    let sessions = deps.sessions.read().await;
    match sessions.get(&user_id) {
        Some(portfolio) => {
            let mut text = String::from("Recent Trades\n─────────────\n");

            if portfolio.trade_history.is_empty() && portfolio.open_positions.is_empty() {
                text.push_str("\nNo trades yet. The bot is scanning for opportunities...");
            }

            if !portfolio.open_positions.is_empty() {
                text.push_str(&format!(
                    "\nOpen Positions ({}):\n",
                    portfolio.open_positions.len()
                ));
                for (_, pos) in top_and_bottom_positions(portfolio, 10) {
                    let end = end_dates.get(&pos.condition_id).copied();
                    text.push_str(&format_open_position(pos, end));
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
                        "{} {} | {}{:.2} ({:.1}%)\n",
                        emoji,
                        strip_imported_prefix(&trade.outcome_name),
                        pnl_sign,
                        trade.realized_pnl,
                        trade.realized_pnl_pct,
                    ));
                }
            }

            let (paused, testing, is_live) = menu_state(&deps).await;
            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
    let (paused, testing, is_live) = menu_state(&deps).await;
    bot.send_message(msg.chat.id, text)
        .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
/// In live mode, return the actual CLOB balance; fall back to PAPER_BALANCE in paper mode.
async fn live_balance_or_paper(deps: &BotDeps) -> Decimal {
    let cfg = deps.config.read().await;
    if cfg.trading_mode == TradingMode::Live {
        if let Some(ref executor) = deps.live_executor {
            if let Some(bal) = executor.clob_balance().await {
                return bal;
            }
        }
    }
    cfg.paper_balance
}

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
                    let (paused, testing, is_live) = menu_state(&deps).await;
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
            let (paused, testing, is_live) = menu_state(&deps).await;

            if markets.is_empty() {
                bot.send_message(chat_id, "No markets currently being tracked.\nThe collector may still be initializing...")
                    .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
                    .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                    .await?;
            }
        }
        "cmd:trades" => {
            let end_dates = build_end_date_map(&deps).await;
            let sessions = deps.sessions.read().await;
            match sessions.get(&user_id) {
                Some(portfolio) => {
                    let mut text = String::from("Recent Trades\n─────────────\n");

                    if portfolio.trade_history.is_empty() && portfolio.open_positions.is_empty() {
                        text.push_str("\nNo trades yet. The bot is scanning for opportunities...");
                    }

                    if !portfolio.open_positions.is_empty() {
                        text.push_str(&format!(
                            "\nOpen Positions ({}):\n",
                            portfolio.open_positions.len()
                        ));
                        for (_, pos) in top_and_bottom_positions(portfolio, 10) {
                            let end = end_dates.get(&pos.condition_id).copied();
                            text.push_str(&format_open_position(pos, end));
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
                                "{} {} | {}{:.2} ({:.1}%)\n",
                                emoji,
                                strip_imported_prefix(&trade.outcome_name),
                                pnl_sign,
                                trade.realized_pnl,
                                trade.realized_pnl_pct,
                            ));
                        }
                    }

                    let (paused, testing, is_live) = menu_state(&deps).await;
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
            let (paused, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, text)
                .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                .await?;
        }
        "cmd:menu" => {
            let (paused, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "Main Menu")
                .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
            let (_, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "Trading paused.")
                .reply_markup(keyboards::main_menu_with_state(true, testing, is_live))
                .await?;
        }
        "confirm:resume" => {
            deps.config.write().await.trading_paused = false;
            info!("User {} resumed trading", user_id);
            let (_, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "Trading resumed!")
                .reply_markup(keyboards::main_menu_with_state(false, testing, is_live))
                .await?;
        }
        "confirm:reset" => {
            let initial_balance = live_balance_or_paper(&deps).await;
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
            let (paused, testing, is_live) = menu_state(&deps).await;
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
            .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
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
        "ask:sell_trade" => {
            let sessions = deps.sessions.read().await;
            match sessions.get(&user_id) {
                Some(portfolio) if !portfolio.open_positions.is_empty() => {
                    let sellable: Vec<_> = top_and_bottom_positions(portfolio, 10)
                        .into_iter()
                        .filter(|(_, pos)| !pos.imported)
                        .collect();
                    let imported_count = portfolio.open_positions.values().filter(|p| p.imported).count();
                    let mut rows: Vec<Vec<InlineKeyboardButton>> = sellable
                        .into_iter()
                        .map(|(pos_id, pos)| {
                            let pnl = pos.unrealized_pnl();
                            let pnl_pct = pos.unrealized_pnl_pct();
                            let sign = if pnl >= Decimal::ZERO { "+" } else { "" };
                            let emoji = if pnl >= Decimal::ZERO { "\u{1f7e2}" } else { "\u{1f534}" };
                            let label = format!(
                                "{} {} | {}{:.2} ({}{:.1}%)",
                                emoji,
                                strip_imported_prefix(&pos.outcome_name),
                                sign, pnl, sign, pnl_pct,
                            );
                            vec![InlineKeyboardButton::callback(label, format!("sell:{pos_id}"))]
                        })
                        .collect();
                    rows.push(vec![InlineKeyboardButton::callback("Cancel", "cmd:menu")]);
                    let note = if imported_count > 0 {
                        format!(
                            "\n\n\u{26a0}\u{fe0f} {imported_count} imported position(s) not shown \
                             — these can only be sold via the Polymarket UI."
                        )
                    } else {
                        String::new()
                    };
                    bot.send_message(
                        chat_id,
                        format!(
                            "Select a position to sell ({} total):\n\
                             Top 10 winners, then top 10 losers{note}",
                            portfolio.open_positions.len()
                        ),
                    )
                    .reply_markup(InlineKeyboardMarkup::new(rows))
                    .await?;
                }
                Some(_) => {
                    bot.send_message(chat_id, "No open positions to sell.").await?;
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.").await?;
                }
            }
        }
        "ask:withdraw" => {
            let sessions = deps.sessions.read().await;
            let pos_count = sessions.get(&user_id).map(|p| p.num_open_positions()).unwrap_or(0);
            bot.send_message(
                chat_id,
                format!(
                    "Withdraw All\n\n\
                     This will:\n\
                     \u{2022} Pause trading\n\
                     \u{2022} Attempt to sell all {} open positions on Polymarket\n\
                     \u{2022} Clear portfolio tracking\n\n\
                     USDC will be returned to your wallet as sell orders fill.\n\
                     Note: low-price positions may not find buyers immediately.\n\n\
                     Are you sure?",
                    pos_count
                ),
            )
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("Yes, Withdraw", "confirm:withdraw"),
                InlineKeyboardButton::callback("Cancel", "cmd:menu"),
            ]]))
            .await?;
        }
        "confirm:withdraw" => {
            // Pause trading
            deps.config.write().await.trading_paused = true;

            let positions: Vec<(String, rust_decimal::Decimal, rust_decimal::Decimal)> = {
                let sessions = deps.sessions.read().await;
                sessions.get(&user_id).map(|p| {
                    p.open_positions.values()
                        .map(|pos| (pos.token_id.clone(), pos.quantity, pos.current_price))
                        .collect()
                }).unwrap_or_default()
            };

            bot.send_message(chat_id, format!("Paused trading. Attempting to sell {} positions...", positions.len()))
                .await?;

            let mut sold = 0usize;
            let mut failed = 0usize;

            if let Some(ref executor) = deps.live_executor {
                for (token_id, qty, price) in &positions {
                    match executor.sell_position(token_id, *qty, *price).await {
                        Ok(_) => sold += 1,
                        Err(e) => {
                            warn!("Withdraw sell failed for {token_id}: {e}");
                            failed += 1;
                        }
                    }
                }
            }

            // Clear portfolio positions
            {
                let mut sessions = deps.sessions.write().await;
                if let Some(portfolio) = sessions.get_mut(&user_id) {
                    let pos_ids: Vec<String> = portfolio.open_positions.keys().cloned().collect();
                    for pos_id in pos_ids {
                        let price = portfolio.open_positions[&pos_id].current_price;
                        let _ = portfolio.close_position(&pos_id, price, "withdraw");
                    }
                }
            }

            if let Some(ref db) = deps.db {
                let sessions = deps.sessions.read().await;
                if let Some(portfolio) = sessions.get(&user_id) {
                    let _ = db.save_session(portfolio).await;
                }
            }

            let (_, testing, is_live) = menu_state(&deps).await;
            bot.send_message(
                chat_id,
                format!(
                    "Withdraw initiated.\n\n\
                     Sell orders placed: {sold}\n\
                     Failed: {failed}\n\n\
                     Portfolio cleared. Check your Polymarket balance for USDC as sell orders fill.\n\
                     Trading remains paused \u{2014} press Resume when ready."
                ),
            )
            .reply_markup(keyboards::main_menu_with_state(true, testing, is_live))
            .await?;
        }
        _ => {
            if data.starts_with("sell:") {
                let pos_id = &data["sell:".len()..];
                let pos_info = {
                    let sessions = deps.sessions.read().await;
                    sessions.get(&user_id).and_then(|p| p.open_positions.get(pos_id)).map(|pos| {
                        (pos.token_id.clone(), pos.quantity, pos.current_price,
                         strip_imported_prefix(&pos.outcome_name).to_string(), pos.imported)
                    })
                };
                match pos_info {
                    None => {
                        bot.send_message(chat_id, "Position not found.").await?;
                    }
                    Some((token_id, qty, price, name, imported)) => {
                        // Submit live sell (skip for imported positions — CLOB can't sell them)
                        let sell_result = if imported {
                            Some(Err(crate::error::FalkeError::OrderRejected(
                                "Imported position — sell via Polymarket UI or wait for resolution".into()
                            )))
                        } else if let Some(ref executor) = deps.live_executor {
                            Some(executor.sell_position(&token_id, qty, price).await)
                        } else {
                            None
                        };
                        // Close locally
                        {
                            let mut sessions = deps.sessions.write().await;
                            if let Some(portfolio) = sessions.get_mut(&user_id) {
                                let _ = portfolio.close_position(pos_id, price, "manual_sell");
                            }
                        }
                        if let Some(ref db) = deps.db {
                            let sessions = deps.sessions.read().await;
                            if let Some(portfolio) = sessions.get(&user_id) {
                                let _ = db.save_session(portfolio).await;
                            }
                        }
                        let msg = match sell_result {
                            Some(Ok(order_id)) => format!("Sell order placed for {name}\nOrder: {order_id}"),
                            Some(Err(e)) => format!("Sell failed for {name}: {e}"),
                            None => format!("Position {name} closed locally (paper mode)."),
                        };
                        let (paused, testing, is_live) = menu_state(&deps).await;
                        bot.send_message(chat_id, msg)
                            .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                            .await?;
                    }
                }
            } else if data.starts_with("mode:") {
                let (paused, testing, is_live) = menu_state(&deps).await;
                bot.send_message(chat_id, format!("Setting updated: {data}"))
                    .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                    .await?;
            }
        }
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() > max_len { &s[..max_len] } else { s }
}

/// Build a condition_id → end_date lookup from current market data.
async fn build_end_date_map(deps: &BotDeps) -> std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> {
    let md = deps.market_data.read().await;
    md.tracked_markets
        .iter()
        .filter_map(|m| m.end_date.map(|d| (m.condition_id.clone(), d)))
        .collect()
}

/// Return up to 10 best P&L positions followed by up to 10 worst P&L positions.
/// Ties within each group preserve insertion order.
fn top_and_bottom_positions(
    portfolio: &crate::trading::portfolio::Portfolio,
    n: usize,
) -> Vec<(&String, &crate::trading::portfolio::Position)> {
    let mut all: Vec<(&String, &crate::trading::portfolio::Position)> =
        portfolio.open_positions.iter().collect();
    // Sort descending by unrealized P&L
    all.sort_by(|a, b| b.1.unrealized_pnl().cmp(&a.1.unrealized_pnl()));
    let top: Vec<_> = all.iter().take(n).cloned().collect();
    // Last `n` are the worst (already sorted descending, so take from the end)
    let bottom_start = if all.len() > n { all.len() - n } else { top.len() };
    let bottom: Vec<_> = all[bottom_start..].iter().rev().cloned().collect();
    // Combine: top winners first, then worst losers; deduplicate overlaps
    let top_ids: std::collections::HashSet<&String> = top.iter().map(|(id, _)| *id).collect();
    let mut result = top;
    for entry in bottom {
        if !top_ids.contains(entry.0) {
            result.push(entry);
        }
    }
    result
}

/// Strip "Imported " prefix added by reconciliation, leave the hex id.
fn strip_imported_prefix(name: &str) -> &str {
    name.strip_prefix("Imported ").unwrap_or(name)
}

/// Format a single open position line: emoji + id + P&L + time to resolve
fn format_open_position(
    pos: &crate::trading::portfolio::Position,
    end_date: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    let pnl = pos.unrealized_pnl();
    let pnl_pct = pos.unrealized_pnl_pct();
    let emoji = if pnl >= rust_decimal::Decimal::ZERO { "\u{1f7e2}" } else { "\u{1f534}" };
    let sign = if pnl >= rust_decimal::Decimal::ZERO { "+" } else { "" };
    let time_str = match end_date {
        Some(end) => {
            let secs = (end - chrono::Utc::now()).num_seconds().max(0);
            let d = secs / 86400;
            let h = (secs % 86400) / 3600;
            let m = (secs % 3600) / 60;
            let s = secs % 60;
            if d > 0 {
                format!(" \u{2192} {}d {:02}:{:02}:{:02}", d, h, m, s)
            } else {
                format!(" \u{2192} {:02}:{:02}:{:02}", h, m, s)
            }
        }
        None => String::new(),
    };
    format!(
        "  {} {}{} | {}{:.2} ({}{:.1}%)\n",
        emoji,
        strip_imported_prefix(&pos.outcome_name),
        time_str,
        sign, pnl, sign, pnl_pct,
    )
}
