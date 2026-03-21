use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, MediaKind, MessageKind, ParseMode, ReplyMarkup,
};
use tracing::{info, warn};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::config::{Config, SharedConfig, TradingMode};
use crate::db::models::GlobalSettings;
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
    (
        cfg.trading_paused,
        cfg.testing_mode,
        cfg.trading_mode == TradingMode::Live,
    )
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

    let tp_count = portfolio
        .trade_history
        .iter()
        .filter(|t| t.close_reason == "take_profit")
        .count();
    let sl_count = portfolio
        .trade_history
        .iter()
        .filter(|t| t.close_reason == "stop_loss")
        .count();
    let resolved_win_count = portfolio
        .trade_history
        .iter()
        .filter(|t| t.close_reason == "resolved_win")
        .count();
    let resolved_loss_count = portfolio
        .trade_history
        .iter()
        .filter(|t| t.close_reason == "resolved_loss")
        .count();

    let winning = portfolio
        .trade_history
        .iter()
        .filter(|t| t.realized_pnl > Decimal::ZERO)
        .count();
    let total_trades = portfolio.trade_history.len();
    let win_rate = if total_trades > 0 {
        format!("{:.0}%", winning as f64 / total_trades as f64 * 100.0)
    } else {
        "N/A".to_string()
    };

    let mode_str = match &config.trading_mode {
        TradingMode::Paper => "Paper Trading",
        TradingMode::Live => "LIVE Trading",
    };
    let mode_str = if config.trading_paused {
        format!("{mode_str} (paused)")
    } else {
        mode_str.to_string()
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

    // Strategy display
    let strategy_line = if config.mean_reversion_budget_pct >= dec!(1.0) {
        format!(
            "Strategy: MR (thr={}% bet=${})",
            config.mean_reversion_threshold * dec!(100),
            config.mean_reversion_bet_usd,
        )
    } else if config.mean_reversion_budget_pct <= Decimal::ZERO {
        format!(
            "Strategy: ML (thr={}% bet=${})",
            config.ml_win_prob_threshold * 100.0,
            config.mean_reversion_bet_usd,
        )
    } else {
        let mr_pct = config.mean_reversion_budget_pct * dec!(100);
        format!(
            "Strategy: ML + MR {:.0}% (thr={}% bet=${})",
            mr_pct,
            config.mean_reversion_threshold * dec!(100),
            config.mean_reversion_bet_usd,
        )
    };

    let ignored_line = if config.ignored_topics.is_empty() {
        "Ignored topics: none".to_string()
    } else {
        format!("Ignored topics: {}", config.ignored_topics.join(", "))
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
         \n\
         Trades: {} (TP: {} / SL: {} / Win: {} / Loss: {})\n\
         Win rate: {win_rate}\n\
         {strategy_line}\n\
         {ignored_line}\n\
         Max bet: ${} | Max pos: {}\n\
         Brake: {}% loss → pause {}min",
        portfolio.balance,
        cash_pct,
        positions_value,
        pos_pct,
        portfolio.num_open_positions(),
        total,
        total_trades,
        tp_count,
        sl_count,
        resolved_win_count,
        resolved_loss_count,
        config.max_bet_usd,
        config.max_open_positions,
        config.budget_brake_pct,
        config.budget_brake_time_sec / 60,
    )
}

/// Build test leaderboard text + list of (rank, portfolio_idx, strategy_desc) for keyboard.
async fn build_test_leaderboard(deps: &BotDeps) -> (String, Vec<(usize, usize, String)>) {
    let ts = match &deps.test_sessions {
        Some(ts) => ts,
        None => return ("Testing mode is not enabled.".to_string(), vec![]),
    };

    let ts_lock = ts.read().await;
    if ts_lock.is_empty() {
        return ("No test portfolios initialized.".to_string(), vec![]);
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
    let mut entries: Vec<(usize, usize, String)> = Vec::new();

    for (rank, (pnl_pct, idx)) in ranked.iter().take(10).enumerate() {
        let tp = &ts_lock[*idx];
        let pnl_usd = tp.portfolio.total_value() - tp.portfolio.initial_balance;
        let sign = if *pnl_pct >= Decimal::ZERO { "+" } else { "" };
        let wins = tp
            .portfolio
            .trade_history
            .iter()
            .filter(|t| t.realized_pnl > Decimal::ZERO)
            .count();
        let losses = tp.portfolio.trade_history.len() - wins;

        let (tag, strategy_desc) = match tp.config.strategy {
            crate::trading::testing::TestStrategy::Mr => (
                "MR",
                format!("thr={:.0}% bet=${:.1}", tp.config.threshold * 100.0, tp.config.bet_usd),
            ),
            crate::trading::testing::TestStrategy::Ml => (
                "ML",
                format!("prob={:.0}% bet=${:.1}", tp.config.threshold * 100.0, tp.config.bet_usd),
            ),
        };

        text.push_str(&format!(
            "{}. [{}] {} | {}{:.1}% (${:.2}) | {} trades W:{} L:{}\n",
            rank + 1,
            tag,
            strategy_desc,
            sign,
            pnl_pct,
            pnl_usd,
            tp.portfolio.trade_history.len(),
            wins,
            losses,
        ));

        entries.push((rank + 1, *idx, strategy_desc));
    }

    (text, entries)
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
            .map(|o| {
                format!(
                    "{}: {:.1}c",
                    o.name,
                    o.price * rust_decimal_macros::dec!(100)
                )
            })
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
    let slugs = build_slug_map(&deps).await;
    let (paused, testing, is_live) = menu_state(&deps).await;
    let sessions = deps.sessions.read().await;
    match sessions.get(&user_id) {
        Some(portfolio) => {
            let chunks = build_trades_chunks(portfolio, &end_dates, &slugs);
            let last = chunks.len().saturating_sub(1);
            let markup = keyboards::main_menu_with_state(paused, testing, is_live);
            for (i, chunk) in chunks.into_iter().enumerate() {
                if i == last {
                    bot.send_message(msg.chat.id, chunk)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(markup.clone())
                        .await?;
                } else {
                    bot.send_message(msg.chat.id, chunk)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
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
    let (text, entries) = build_test_leaderboard(&deps).await;
    let kb = test_leaderboard_keyboard(&entries);
    bot.send_message(msg.chat.id, text)
        .reply_markup(kb)
        .await?;
    Ok(())
}

/// Build inline keyboard for the test leaderboard: one 📊 button per entry.
fn test_leaderboard_keyboard(
    entries: &[(usize, usize, String)],
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = entries
        .chunks(5)
        .map(|row| {
            row.iter()
                .map(|(rank, idx, _)| {
                    InlineKeyboardButton::callback(
                        format!("\u{1f4ca} #{rank}"),
                        format!("test:trades:{idx}"),
                    )
                })
                .collect()
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "\u{2190} Menu",
        "cmd:menu",
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// Handle /stop command
pub async fn handle_stop(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let (_, _, is_live) = menu_state(&deps).await;
    bot.send_message(msg.chat.id, "What would you like to do?")
        .reply_markup(keyboards::stop_menu(is_live))
        .await?;
    Ok(())
}

/// Handle callback queries from inline keyboards
/// In live mode, return the actual CLOB balance; fall back to PAPER_BALANCE in paper mode.
/// Persist current managed settings to DynamoDB (fire-and-forget, logs on error).
async fn save_settings_to_db(deps: &BotDeps) {
    if let Some(ref db) = deps.db {
        let cfg = deps.config.read().await;
        let s = GlobalSettings {
            paused: cfg.trading_paused,
            trading_mode: Some(match cfg.trading_mode {
                crate::config::TradingMode::Live => "live".into(),
                crate::config::TradingMode::Paper => "paper".into(),
            }),
            market_expiry_window_hours: Some(cfg.market_expiry_window_hours),
            max_open_positions: Some(cfg.max_open_positions),
        };
        drop(cfg);
        if let Err(e) = db.save_global_settings(&s).await {
            warn!("Failed to persist global settings: {e}");
        }
    }
}

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
                        .map(|o| {
                            format!(
                                "{}: {:.1}c",
                                o.name,
                                o.price * rust_decimal_macros::dec!(100)
                            )
                        })
                        .collect();
                    let price_sum: rust_decimal::Decimal =
                        market.outcomes.iter().map(|o| o.price).sum();
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
                    .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                    .await?;
            }
        }
        "cmd:trades" => {
            let end_dates = build_end_date_map(&deps).await;
            let slugs = build_slug_map(&deps).await;
            let (paused, testing, is_live) = menu_state(&deps).await;
            let sessions = deps.sessions.read().await;
            match sessions.get(&user_id) {
                Some(portfolio) => {
                    let chunks = build_trades_chunks(portfolio, &end_dates, &slugs);
                    let last = chunks.len().saturating_sub(1);
                    let markup = keyboards::main_menu_with_state(paused, testing, is_live);
                    for (i, chunk) in chunks.into_iter().enumerate() {
                        if i == last {
                            bot.send_message(chat_id, chunk)
                                .parse_mode(ParseMode::Html)
                                .reply_markup(markup.clone())
                                .await?;
                        } else {
                            bot.send_message(chat_id, chunk)
                                .parse_mode(ParseMode::Html)
                                .await?;
                        }
                    }
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.")
                        .await?;
                }
            }
        }
        "cmd:test" => {
            let (text, entries) = build_test_leaderboard(&deps).await;
            let kb = test_leaderboard_keyboard(&entries);
            bot.send_message(chat_id, text).reply_markup(kb).await?;
        }
        "cmd:menu" => {
            let (paused, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "Main Menu")
                .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                .await?;
        }
        "cmd:stop" => {
            let (_, _, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "What would you like to do?")
                .reply_markup(keyboards::stop_menu(is_live))
                .await?;
        }
        "confirm:stop" => {
            deps.config.write().await.trading_paused = true;
            info!("User {} paused trading", user_id);
            save_settings_to_db(&deps).await;
            let (_, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "Trading paused.")
                .reply_markup(keyboards::main_menu_with_state(true, testing, is_live))
                .await?;
        }
        "confirm:resume" => {
            deps.config.write().await.trading_paused = false;
            info!("User {} resumed trading", user_id);
            save_settings_to_db(&deps).await;
            let (_, testing, is_live) = menu_state(&deps).await;
            bot.send_message(chat_id, "Trading resumed!")
                .reply_markup(keyboards::main_menu_with_state(false, testing, is_live))
                .await?;
            // Reconcile live positions against CLOB when resuming in live mode
            if is_live {
                if let Some(ref executor) = deps.live_executor {
                    info!("Reconciling live positions on resume for user {}", user_id);
                    crate::trading::reconcile::reconcile_live_positions(
                        executor.clob(),
                        &deps.sessions,
                        &bot,
                    )
                    .await;
                }
            }
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
            // Reset test portfolios to fresh state
            if let Some(ref ts) = deps.test_sessions {
                let cfg = deps.config.read().await;
                let fresh = crate::trading::testing::generate_test_portfolios(&cfg);
                drop(cfg);
                if let Some(ref db) = deps.db {
                    for tp in &fresh {
                        if let Err(e) = db.save_session(&tp.portfolio).await {
                            warn!("Failed to reset test session {}: {e}", tp.config.name);
                        }
                    }
                }
                *ts.write().await = fresh;
                info!("Test portfolios reset to fresh state");
            }
            info!(
                "User {} reset paper session to ${}",
                user_id, initial_balance
            );
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
                    let imported_count = portfolio
                        .open_positions
                        .values()
                        .filter(|p| p.imported)
                        .count();
                    let mut rows: Vec<Vec<InlineKeyboardButton>> = sellable
                        .into_iter()
                        .map(|(pos_id, pos)| {
                            let pnl = pos.unrealized_pnl();
                            let pnl_pct = pos.unrealized_pnl_pct();
                            let sign = if pnl >= Decimal::ZERO { "+" } else { "" };
                            let emoji = if pnl >= Decimal::ZERO {
                                "\u{1f7e2}"
                            } else {
                                "\u{1f534}"
                            };
                            let label = format!(
                                "{} {} | {}{:.2} ({}{:.1}%)",
                                emoji,
                                strip_imported_prefix(&pos.outcome_name),
                                sign,
                                pnl,
                                sign,
                                pnl_pct,
                            );
                            vec![InlineKeyboardButton::callback(
                                label,
                                format!("sell:{pos_id}"),
                            )]
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
                    bot.send_message(chat_id, "No open positions to sell.")
                        .await?;
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.")
                        .await?;
                }
            }
        }
        "ask:withdraw" => {
            let sessions = deps.sessions.read().await;
            let pos_count = sessions
                .get(&user_id)
                .map(|p| p.num_open_positions())
                .unwrap_or(0);
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
                sessions
                    .get(&user_id)
                    .map(|p| {
                        p.open_positions
                            .values()
                            .map(|pos| (pos.token_id.clone(), pos.quantity, pos.current_price))
                            .collect()
                    })
                    .unwrap_or_default()
            };

            bot.send_message(
                chat_id,
                format!(
                    "Paused trading. Attempting to sell {} positions...",
                    positions.len()
                ),
            )
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
        "cmd:settings" => {
            let cfg = deps.config.read().await;
            let is_mr = cfg.mean_reversion_budget_pct >= dec!(1.0);
            let text = keyboards::settings_text(
                cfg.market_expiry_window_hours,
                cfg.max_open_positions,
                cfg.trading_paused,
                cfg.mean_reversion_budget_pct,
                cfg.mean_reversion_threshold,
                cfg.mean_reversion_bet_usd,
            );
            let kb = keyboards::settings_keyboard(cfg.trading_paused, is_mr);
            drop(cfg);
            bot.send_message(chat_id, text).reply_markup(kb).await?;
        }
        _ => {
            if data.starts_with("settings:") {
                let action = &data["settings:".len()..];
                {
                    let mut cfg = deps.config.write().await;
                    match action {
                        "mr_bet_up" => cfg.mean_reversion_bet_usd += dec!(1),
                        "mr_bet_down" => {
                            cfg.mean_reversion_bet_usd =
                                (cfg.mean_reversion_bet_usd - dec!(1)).max(dec!(1))
                        }
                        "mr_thr_up" => {
                            cfg.mean_reversion_threshold =
                                (cfg.mean_reversion_threshold + dec!(0.05)).min(dec!(0.99))
                        }
                        "mr_thr_down" => {
                            cfg.mean_reversion_threshold =
                                (cfg.mean_reversion_threshold - dec!(0.05)).max(dec!(0.05))
                        }
                        "window_up" => cfg.market_expiry_window_hours += 1,
                        "window_down" => {
                            cfg.market_expiry_window_hours =
                                cfg.market_expiry_window_hours.saturating_sub(1).max(1)
                        }
                        "positions_up" => cfg.max_open_positions += 10,
                        "positions_down" => {
                            cfg.max_open_positions =
                                cfg.max_open_positions.saturating_sub(10).max(1)
                        }
                        _ => {}
                    }
                }
                save_settings_to_db(&deps).await;
                let cfg = deps.config.read().await;
                let is_mr = cfg.mean_reversion_budget_pct >= dec!(1.0);
                let text = keyboards::settings_text(
                    cfg.market_expiry_window_hours,
                    cfg.max_open_positions,
                    cfg.trading_paused,
                    cfg.mean_reversion_budget_pct,
                    cfg.mean_reversion_threshold,
                    cfg.mean_reversion_bet_usd,
                );
                let kb = keyboards::settings_keyboard(cfg.trading_paused, is_mr);
                drop(cfg);
                bot.send_message(chat_id, text).reply_markup(kb).await?;
            } else if data.starts_with("sell:") {
                let pos_id = &data["sell:".len()..];
                let pos_info = {
                    let sessions = deps.sessions.read().await;
                    sessions
                        .get(&user_id)
                        .and_then(|p| p.open_positions.get(pos_id))
                        .map(|pos| {
                            (
                                pos.token_id.clone(),
                                pos.quantity,
                                pos.current_price,
                                strip_imported_prefix(&pos.outcome_name).to_string(),
                                pos.imported,
                            )
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
                                "Imported position — sell via Polymarket UI or wait for resolution"
                                    .into(),
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
                            Some(Ok(order_id)) => {
                                format!("Sell order placed for {name}\nOrder: {order_id}")
                            }
                            Some(Err(e)) => format!("Sell failed for {name}: {e}"),
                            None => format!("Position {name} closed locally (paper mode)."),
                        };
                        let (paused, testing, is_live) = menu_state(&deps).await;
                        bot.send_message(chat_id, msg)
                            .reply_markup(keyboards::main_menu_with_state(paused, testing, is_live))
                            .await?;
                    }
                }
            } else if data.starts_with("test:trades:") {
                let idx_str = &data["test:trades:".len()..];
                let idx: usize = idx_str.parse().unwrap_or(usize::MAX);
                let result = if let Some(ref ts) = deps.test_sessions {
                    let ts_lock = ts.read().await;
                    ts_lock.get(idx).map(|tp| {
                        let desc = match tp.config.strategy {
                            crate::trading::testing::TestStrategy::Mr => format!(
                                "Test Portfolio: [MR] thr={:.0}% bet=${:.1}\n─────────────────\n",
                                tp.config.threshold * 100.0,
                                tp.config.bet_usd,
                            ),
                            crate::trading::testing::TestStrategy::Ml => format!(
                                "Test Portfolio: [ML] prob={:.0}% bet=${:.1}\n─────────────────\n",
                                tp.config.threshold * 100.0,
                                tp.config.bet_usd,
                            ),
                        };
                        (tp.portfolio.clone(), desc)
                    })
                } else {
                    None
                };
                match result {
                    None => {
                        bot.send_message(chat_id, "Test portfolio not found.").await?;
                    }
                    Some((portfolio, header)) => {
                        let end_dates = build_end_date_map(&deps).await;
                        let slugs = build_slug_map(&deps).await;
                        let mut chunks = build_trades_chunks(&portfolio, &end_dates, &slugs);
                        if let Some(first) = chunks.first_mut() {
                            *first = format!("{header}{first}");
                        }
                        let back_kb = InlineKeyboardMarkup::new(vec![vec![
                            InlineKeyboardButton::callback("\u{2190} Leaderboard", "cmd:test"),
                        ]]);
                        let last = chunks.len().saturating_sub(1);
                        for (i, chunk) in chunks.into_iter().enumerate() {
                            if i == last {
                                bot.send_message(chat_id, chunk)
                                    .parse_mode(ParseMode::Html)
                                    .reply_markup(back_kb.clone())
                                    .await?;
                            } else {
                                bot.send_message(chat_id, chunk)
                                    .parse_mode(ParseMode::Html)
                                    .await?;
                            }
                        }
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
    if s.len() > max_len {
        &s[..max_len]
    } else {
        s
    }
}

/// Build the full trades view as Telegram-safe chunks (≤4096 chars each).
/// Layout: open positions (all, sorted by P&L desc) → closed trades (≤300, newest first) → summary.
fn build_trades_chunks(
    portfolio: &crate::trading::portfolio::Portfolio,
    end_dates: &std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    slugs: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut text = String::new();

    if portfolio.trade_history.is_empty() && portfolio.open_positions.is_empty() {
        text.push_str("No trades yet. The bot is scanning for opportunities...");
        return vec![text];
    }

    // --- Open positions: all, sorted by unrealized P&L descending ---
    if !portfolio.open_positions.is_empty() {
        let pos_total: rust_decimal::Decimal = portfolio
            .open_positions
            .values()
            .map(|p| p.current_price * p.quantity)
            .sum();
        let unrealized_total = portfolio.total_unrealized_pnl();
        let u_sign = if unrealized_total >= rust_decimal::Decimal::ZERO {
            "+"
        } else {
            ""
        };
        text.push_str(&format!(
            "Open Positions ({}) | ${:.2} ({}{:.2} P&L):\n",
            portfolio.open_positions.len(),
            pos_total,
            u_sign,
            unrealized_total,
        ));
        let mut sorted: Vec<_> = portfolio.open_positions.iter().collect();
        sorted.sort_by(|a, b| b.1.unrealized_pnl().cmp(&a.1.unrealized_pnl()));
        for (_, pos) in sorted {
            let end = end_dates.get(&pos.condition_id).copied();
            let slug = pos
                .market_url
                .as_deref()
                .or_else(|| slugs.get(&pos.condition_id).map(|s| s.as_str()));
            text.push_str(&format_open_position(pos, end, slug));
        }
    }

    // --- Closed trades: newest first, limit 50 (more = multiple API calls = slow) ---
    const MAX_CLOSED: usize = 50;
    // Deduplicate by (token_id, entry_price, exit_price, quantity) — more robust than
    // using realized_pnl which may have different Decimal scale from arithmetic vs parsing.
    let mut seen_trades = std::collections::HashSet::new();
    let closed: Vec<_> = portfolio
        .trade_history
        .iter()
        .rev()
        .filter(|t| {
            seen_trades.insert(format!(
                "{};{:.4};{:.4};{:.4}",
                t.token_id, t.entry_price, t.exit_price, t.quantity
            ))
        })
        .take(MAX_CLOSED)
        .collect();
    if !closed.is_empty() {
        let total_closed = portfolio.trade_history.len();
        let showing = total_closed.min(MAX_CLOSED);
        let closed_header = if total_closed > MAX_CLOSED {
            format!("\nClosed (showing {} of {}):\n", showing, total_closed)
        } else {
            format!("\nClosed ({}):\n", total_closed)
        };
        text.push_str(&closed_header);
        for trade in &closed {
            let (emoji, sign) = if trade.realized_pnl >= rust_decimal::Decimal::ZERO {
                ("\u{1f7e2}", "+")
            } else {
                ("\u{1f534}", "")
            };
            let label = he(strip_imported_prefix(&trade.outcome_name));
            let id_part = match trade
                .market_url
                .as_deref()
                .or_else(|| slugs.get(&trade.condition_id).map(|s| s.as_str()))
            {
                Some(url_path) => format!(
                    "<a href=\"https://polymarket.com/event/{}\">{}</a>",
                    he(url_path),
                    label
                ),
                None => label,
            };
            text.push_str(&format!(
                "{} {} | {}{:.2} ({:.1}%)\n",
                emoji, id_part, sign, trade.realized_pnl, trade.realized_pnl_pct,
            ));
        }
    }

    // --- Summary P&L ---
    let realized: rust_decimal::Decimal =
        portfolio.trade_history.iter().map(|t| t.realized_pnl).sum();
    let unrealized = portfolio.total_unrealized_pnl();
    let total_pnl = realized + unrealized;
    let r_sign = if realized >= rust_decimal::Decimal::ZERO {
        "+"
    } else {
        ""
    };
    let u_sign = if unrealized >= rust_decimal::Decimal::ZERO {
        "+"
    } else {
        ""
    };
    let t_sign = if total_pnl >= rust_decimal::Decimal::ZERO {
        "+"
    } else {
        ""
    };
    text.push_str(&format!(
        "\nSummary P&L:\n  Realized:   {r_sign}{realized:.2}\n  Unrealized: {u_sign}{unrealized:.2}\n  Total:      {t_sign}{total_pnl:.2}"
    ));

    // Split into ≤4096 byte chunks (Telegram limit).
    // Must split on a char boundary to avoid panics with multi-byte characters (e.g. '…').
    let mut chunks = Vec::new();
    let mut remaining = text.as_str();
    while !remaining.is_empty() {
        if remaining.len() <= 4096 {
            chunks.push(remaining.to_string());
            break;
        }
        // Find the last char boundary at or before byte 4096
        let boundary = (0..=4096)
            .rev()
            .find(|&i| remaining.is_char_boundary(i))
            .unwrap_or(0);
        // Split at last newline before that boundary
        let split = remaining[..boundary].rfind('\n').unwrap_or(boundary);
        if split == 0 {
            // No newline found — force-split at boundary to avoid infinite loop
            chunks.push(remaining[..boundary].to_string());
            remaining = &remaining[boundary..];
        } else {
            chunks.push(remaining[..split].to_string());
            remaining = &remaining[split..];
        }
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

/// Build a condition_id → end_date lookup from current market data.
async fn build_end_date_map(
    deps: &BotDeps,
) -> std::collections::HashMap<String, chrono::DateTime<chrono::Utc>> {
    let md = deps.market_data.read().await;
    md.tracked_markets
        .iter()
        .filter_map(|m| m.end_date.map(|d| (m.condition_id.clone(), d)))
        .collect()
}

/// Build a condition_id → Polymarket URL path lookup from the persistent slug cache.
async fn build_slug_map(deps: &BotDeps) -> std::collections::HashMap<String, String> {
    let md = deps.market_data.read().await;
    md.slug_cache.clone()
}

/// HTML-escape a string for use in Telegram HTML parse mode.
fn he(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
    let bottom_start = if all.len() > n {
        all.len() - n
    } else {
        top.len()
    };
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
    slug: Option<&str>,
) -> String {
    let pnl = pos.unrealized_pnl();
    let pnl_pct = pos.unrealized_pnl_pct();
    let emoji = if pnl >= rust_decimal::Decimal::ZERO {
        "\u{1f7e2}"
    } else {
        "\u{1f534}"
    };
    let sign = if pnl >= rust_decimal::Decimal::ZERO {
        "+"
    } else {
        ""
    };
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
    let label = he(strip_imported_prefix(&pos.outcome_name));
    let id_part = if let Some(s) = slug {
        format!(
            "<a href=\"https://polymarket.com/event/{}\">{}</a>",
            he(s),
            label
        )
    } else {
        label
    };
    format!(
        "{} {}{} | {}{:.2} ({}{:.1}%)\n",
        emoji, id_part, time_str, sign, pnl, sign, pnl_pct,
    )
}
