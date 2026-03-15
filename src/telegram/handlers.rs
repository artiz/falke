use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{MediaKind, MessageKind, ReplyMarkup};
use tracing::{info, warn};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::config::{Config, SharedConfig, TradingMode};
use crate::market_data::collector::SharedMarketData;
use crate::strategy::signals::SignalSource;
use crate::trading::engine::{SharedDb, SharedSessions};
use crate::trading::portfolio::Portfolio;

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
}

/// Handle the /start command — initiate registration
pub async fn handle_start(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    // Check if already registered
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
            bot.send_message(msg.chat.id, msg_text)
                .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
        let initial_balance = deps.config.read().await.paper_balance;
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
        .reply_markup(ReplyMarkup::InlineKeyboard(keyboards::main_menu_with_state(deps.config.read().await.trading_paused)))
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
    let mode_str = match (&config.trading_mode, config.trading_paused) {
        (_, true) => "PAUSED",
        (TradingMode::Paper, false) => "Paper Trading",
        (TradingMode::Live, false) => "LIVE Trading",
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
            let text = build_portfolio_text(portfolio, &*deps.config.read().await);
            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
        .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
                        "  [{}] {} {} @ {:.2}c | P&L: ${:.2}\n",
                        source_label(&pos.source),
                        pos.side,
                        truncate(&pos.outcome_name, 20),
                        pos.entry_price * rust_decimal_macros::dec!(100),
                        pos.unrealized_pnl(),
                    ));
                }
            }

            // Recent closed trades
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
                        "{} [{}] {} {} | {}{:.2} ({:.1}%)\n",
                        emoji,
                        source_label(&trade.source),
                        trade.side,
                        truncate(&trade.outcome_name, 20),
                        pnl_sign,
                        trade.realized_pnl,
                        trade.realized_pnl_pct,
                    ));
                }
            }

            bot.send_message(msg.chat.id, text)
                .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
                .await?;
        }
        None => {
            bot.send_message(msg.chat.id, "You're not registered. Use /start to begin.")
                .await?;
        }
    }

    Ok(())
}

fn build_strategy_text(config: &Config) -> String {
    format!(
        "Strategy Configuration\n\
         ─────────────────────\n\n\
         Arb: {}% | Mom: {}% | MR: {}% | Tail: {}%\n\n\
         Mom threshold: {}% (5min)\n\
         MR threshold: {}% (fade spikes)\n\
         Tail max price: {}c (${}/ bet)\n\n\
         Risk: TP {}% / SL {}%\n\
         P&L notify: ${}\n\n\
         Choose allocation:",
        config.arb_budget_pct * dec!(100),
        config.momentum_budget_pct * dec!(100),
        config.mean_reversion_budget_pct * dec!(100),
        config.tail_risk_budget_pct * dec!(100),
        config.momentum_derivative_threshold * dec!(100),
        config.mean_reversion_threshold * dec!(100),
        config.tail_risk_max_price * dec!(100),
        config.tail_risk_bet_usd,
        config.take_profit_pct,
        config.stop_loss_pct,
        config.pnl_notify_threshold_usd,
    )
}

/// Handle /strategy command
pub async fn handle_strategy(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    bot.send_message(msg.chat.id, build_strategy_text(&*deps.config.read().await))
        .reply_markup(keyboards::strategy_keyboard())
        .await?;

    Ok(())
}

/// Handle /mode command
pub async fn handle_mode(bot: Bot, msg: Message, deps: BotDeps) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let sessions = deps.sessions.read().await;
    let current_mode = match sessions.get(&user_id) {
        Some(_portfolio) => deps.config.read().await.trading_mode.clone(),
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
                    let text = build_portfolio_text(portfolio, &*deps.config.read().await);
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
                    .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
                    .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
                                "  [{}] {} {} @ {:.2}c | P&L: ${:.2}\n",
                                source_label(&pos.source),
                                pos.side,
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
                                "{} [{}] {} {} | {}{:.2} ({:.1}%)\n",
                                emoji,
                                source_label(&trade.source),
                                trade.side,
                                truncate(&trade.outcome_name, 20),
                                pnl_sign,
                                trade.realized_pnl,
                                trade.realized_pnl_pct,
                            ));
                        }
                    }

                    bot.send_message(chat_id, text)
                        .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
                        .await?;
                }
                None => {
                    bot.send_message(chat_id, "You're not registered. Use /start to begin.")
                        .await?;
                }
            }
        }
        "cmd:strategy" => {
            bot.send_message(chat_id, build_strategy_text(&*deps.config.read().await))
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
                .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
            bot.send_message(chat_id, "Trading paused.")
                .reply_markup(keyboards::main_menu_with_state(true))
                .await?;
        }
        "confirm:resume" => {
            deps.config.write().await.trading_paused = false;
            info!("User {} resumed trading", user_id);
            bot.send_message(chat_id, "Trading resumed!")
                .reply_markup(keyboards::main_menu_with_state(false))
                .await?;
        }
        "confirm:reset" => {
            let initial_balance = deps.config.read().await.paper_balance;
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
            .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
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
            if let Some(preset) = data.strip_prefix("strategy:") {
                let budgets: Option<(Decimal, Decimal, Decimal, Decimal)> = match preset {
                    "balanced"        => Some((dec!(0.10), dec!(0.25), dec!(0.25), dec!(0.20))),
                    "mom_heavy"       => Some((dec!(0.10), dec!(0.35), dec!(0.25), dec!(0.10))),
                    "mr_heavy"        => Some((dec!(0.10), dec!(0.15), dec!(0.35), dec!(0.20))),
                    "tail_heavy"      => Some((dec!(0.10), dec!(0.15), dec!(0.15), dec!(0.40))),
                    "mr_focus"        => Some((dec!(0.00), dec!(0.20), dec!(0.60), dec!(0.20))),
                    "mr_tail"         => Some((dec!(0.00), dec!(0.00), dec!(0.70), dec!(0.30))),
                    "mr_tail_balanced"=> Some((dec!(0.00), dec!(0.00), dec!(0.50), dec!(0.50))),
                    "high_risk"       => Some((dec!(0.00), dec!(0.00), dec!(0.30), dec!(0.70))),
                    "all_tail"        => Some((dec!(0.00), dec!(0.00), dec!(0.00), dec!(1.00))),
                    _ => None,
                };
                if let Some((arb, mom, mr, tail)) = budgets {
                    {
                        let mut cfg = deps.config.write().await;
                        cfg.arb_budget_pct = arb;
                        cfg.momentum_budget_pct = mom;
                        cfg.mean_reversion_budget_pct = mr;
                        cfg.tail_risk_budget_pct = tail;
                    }
                    info!("User {} changed strategy to {preset}: arb={arb} mom={mom} mr={mr} tail={tail}", user_id);
                    let confirm = format!(
                        "Strategy updated: Arb {}% | Mom {}% | MR {}% | Tail {}%",
                        arb * dec!(100), mom * dec!(100), mr * dec!(100), tail * dec!(100),
                    );
                    bot.send_message(chat_id, confirm).await?;
                    bot.send_message(chat_id, build_strategy_text(&*deps.config.read().await))
                        .reply_markup(keyboards::strategy_keyboard())
                        .await?;
                }
            } else if data.starts_with("mode:") {
                bot.send_message(chat_id, format!("Setting updated: {data}"))
                    .reply_markup(keyboards::main_menu_with_state(deps.config.read().await.trading_paused))
                    .await?;
            }
        }
    }

    Ok(())
}

fn source_label(source: &SignalSource) -> &'static str {
    match source {
        SignalSource::Arbitrage => "ARB",
        SignalSource::Momentum => "MOM",
        SignalSource::MeanReversion => "MR",
        SignalSource::TailRisk => "TAIL",
    }
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() > max_len {
        &s[..max_len]
    } else {
        s
    }
}
