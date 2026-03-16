use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ChatId;
use tokio::sync::RwLock;
use tokio::time::{self, Duration};
use tracing::{debug, info, warn};

use crate::config::{SharedConfig, TradingMode};
use crate::db::dynamo::DynamoStore;
use crate::market_data::collector::SharedMarketData;
use crate::strategy::risk::RiskManager;
use crate::strategy::signals::Signal;
use crate::strategy::tail_risk;

use super::executor::LiveExecutor;
use super::paper::PaperTradingEngine;
use super::portfolio::Portfolio;
use super::testing::SharedTestSessions;

/// Shared user sessions — maps telegram_user_id -> Portfolio
pub type SharedSessions = Arc<RwLock<HashMap<i64, Portfolio>>>;

/// Optional shared DynamoDB store
pub type SharedDb = Option<Arc<DynamoStore>>;

pub fn new_shared_sessions() -> SharedSessions {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Save all sessions to DynamoDB
pub async fn save_all_sessions(sessions: &SharedSessions, db: &SharedDb) {
    let db = match db {
        Some(db) => db,
        None => return,
    };

    let sessions_lock = sessions.read().await;
    for portfolio in sessions_lock.values() {
        if let Err(e) = db.save_session(portfolio).await {
            warn!("Failed to save session for user {}: {e}", portfolio.user_id);
        }
    }
}

/// The main trading engine loop.
pub async fn run_engine(
    shared_config: SharedConfig,
    market_data: SharedMarketData,
    sessions: SharedSessions,
    db: SharedDb,
    bot: Bot,
    live_executor: Option<Arc<LiveExecutor>>,
    test_sessions: Option<SharedTestSessions>,
) {
    let (poll_interval, notify_threshold) = {
        let cfg = shared_config.read().await;
        (
            Duration::from_secs(cfg.trade_poll_interval_sec),
            cfg.pnl_notify_threshold_usd,
        )
    };
    let paper_engine = PaperTradingEngine::new();
    let mut risk_manager = RiskManager::new(&*shared_config.read().await);

    // Track last notified P&L level per user (to avoid spam)
    let mut last_notified_pnl: HashMap<i64, Decimal> = HashMap::new();

    // Save sessions every 30 seconds
    let save_interval = Duration::from_secs(30);
    let mut last_save = std::time::Instant::now();

    {
        let cfg = shared_config.read().await;
        info!(
            "Trading engine started in {:?} mode. Poll interval: {}s{}",
            cfg.trading_mode,
            cfg.trade_poll_interval_sec,
            if cfg.testing_mode { " [TESTING MODE]" } else { "" },
        );
    }

    // Give the collector time to fetch initial data
    time::sleep(Duration::from_secs(5)).await;

    loop {
        // Snapshot config for this iteration (picks up any runtime strategy changes)
        let config = shared_config.read().await.clone();

        // 1. Scan for signals (skip when paused)
        if config.trading_paused {
            time::sleep(poll_interval).await;
            continue;
        }

        let all_signals: Vec<Signal> = tail_risk::scan_tail_risk(&config, &market_data).await;

        if !all_signals.is_empty() {
            debug!("Found {} signals this cycle", all_signals.len());
        }

        // 2. Build price map and update all portfolios
        let price_map: HashMap<String, Decimal> = {
            let data = market_data.read().await;
            let mut m = HashMap::new();
            for market in &data.tracked_markets {
                for outcome in &market.outcomes {
                    m.insert(outcome.token_id.clone(), outcome.price);
                }
            }
            m
        };

        {
            let mut sessions_lock = sessions.write().await;
            for portfolio in sessions_lock.values_mut() {
                portfolio.update_prices(&price_map);
            }
        }

        // 2b. Update prices in test portfolios
        if let Some(ref ts) = test_sessions {
            let mut ts_lock = ts.write().await;
            for tp in ts_lock.iter_mut() {
                tp.portfolio.update_prices(&price_map);
            }
        }

        let mut traded = false;

        // 2.5. Auto-exit: check TP/SL on all open positions
        {
            let sl = config.tail_risk_stop_loss_pct;
            let mut sessions_lock = sessions.write().await;
            for portfolio in sessions_lock.values_mut() {
                let position_ids: Vec<String> = portfolio.open_positions.keys().cloned().collect();
                for pos_id in position_ids {
                    let (pnl_pct, current_price, use_take_profit) = {
                        let pos = &portfolio.open_positions[&pos_id];
                        (pos.unrealized_pnl_pct(), pos.current_price, pos.use_take_profit)
                    };
                    if current_price >= dec!(0.99) {
                        match portfolio.close_position(&pos_id, current_price, "resolved_win") {
                            Ok(trade) => {
                                traded = true;
                                info!(
                                    "RESOLVED WIN for user {}: {} | P&L: ${:.2} ({:.1}%)",
                                    portfolio.user_id, trade.outcome_name,
                                    trade.realized_pnl, trade.realized_pnl_pct
                                );
                            }
                            Err(e) => warn!("Failed to close resolved win: {e}"),
                        }
                        continue;
                    } else if current_price <= dec!(0.001) {
                        match portfolio.close_position(&pos_id, current_price, "resolved_loss") {
                            Ok(trade) => {
                                traded = true;
                                info!(
                                    "RESOLVED LOSS for user {}: {} | P&L: ${:.2} ({:.1}%)",
                                    portfolio.user_id, trade.outcome_name,
                                    trade.realized_pnl, trade.realized_pnl_pct
                                );
                            }
                            Err(e) => warn!("Failed to close resolved loss: {e}"),
                        }
                        continue;
                    }

                    if use_take_profit && pnl_pct >= config.tail_risk_take_profit_pct {
                        match portfolio.close_position(&pos_id, current_price, "take_profit") {
                            Ok(trade) => {
                                traded = true;
                                info!(
                                    "TAIL TP for user {}: {} | P&L: ${:.2} ({:.1}%)",
                                    portfolio.user_id, trade.outcome_name,
                                    trade.realized_pnl, trade.realized_pnl_pct
                                );
                            }
                            Err(e) => warn!("Failed to close tail TP: {e}"),
                        }
                    } else if sl > Decimal::ZERO && pnl_pct <= -sl {
                        match portfolio.close_position(&pos_id, current_price, "stop_loss") {
                            Ok(trade) => {
                                traded = true;
                                info!(
                                    "TAIL SL for user {}: {} | P&L: ${:.2} ({:.1}%)",
                                    portfolio.user_id, trade.outcome_name,
                                    trade.realized_pnl, trade.realized_pnl_pct
                                );
                            }
                            Err(e) => warn!("Failed to close tail SL: {e}"),
                        }
                    }
                }
            }
        }

        // 2.6. Auto-exit for test portfolios
        if let Some(ref ts) = test_sessions {
            let sl = config.tail_risk_stop_loss_pct;
            let mut ts_lock = ts.write().await;
            for tp in ts_lock.iter_mut() {
                let position_ids: Vec<String> =
                    tp.portfolio.open_positions.keys().cloned().collect();
                for pos_id in position_ids {
                    let (pnl_pct, current_price) = {
                        let pos = &tp.portfolio.open_positions[&pos_id];
                        (pos.unrealized_pnl_pct(), pos.current_price)
                    };
                    if current_price >= dec!(0.99) {
                        let _ = tp.portfolio.close_position(&pos_id, current_price, "resolved_win");
                    } else if current_price <= dec!(0.001) {
                        let _ = tp.portfolio.close_position(&pos_id, current_price, "resolved_loss");
                    } else if pnl_pct >= tp.config.take_profit_pct {
                        let _ = tp.portfolio.close_position(&pos_id, current_price, "take_profit");
                    } else if sl > Decimal::ZERO && pnl_pct <= -sl {
                        let _ = tp.portfolio.close_position(&pos_id, current_price, "stop_loss");
                    }
                }
            }
        }

        // 3. Execute signals for each active user session
        if !all_signals.is_empty() {
            let mut sessions_lock = sessions.write().await;

            for portfolio in sessions_lock.values_mut() {
                for signal in &all_signals {
                    if portfolio.open_positions.values().any(|p| p.token_id == signal.token_id) {
                        continue;
                    }

                    let open_pos = portfolio.num_open_positions();
                    let balance = portfolio.balance;

                    let amount = match risk_manager.evaluate(signal, balance, open_pos) {
                        Some(a) => a,
                        None => continue,
                    };

                    let use_tp = rand::thread_rng().gen_bool(config.tail_risk_take_profit_fraction);

                    match config.trading_mode {
                        TradingMode::Paper => {
                            match paper_engine.execute_signal(signal, amount, portfolio, use_tp) {
                                Ok(id) => {
                                    risk_manager.record_trade(&signal.token_id);
                                    traded = true;
                                    info!("Paper trade for user {}: {}", portfolio.user_id, id);
                                }
                                Err(e) => {
                                    warn!("Paper trade failed for user {}: {e}", portfolio.user_id);
                                }
                            }
                        }
                        TradingMode::Live => {
                            if let Some(ref executor) = live_executor {
                                match executor.execute_signal(signal, amount).await {
                                    Ok(order_id) => {
                                        risk_manager.record_trade(&signal.token_id);
                                        let _ = paper_engine
                                            .execute_signal(signal, amount, portfolio, use_tp);
                                        traded = true;
                                        info!(
                                            "Live order for user {}: order_id={}",
                                            portfolio.user_id, order_id
                                        );
                                    }
                                    Err(e) => {
                                        warn!("Live order failed for user {}: {e}", portfolio.user_id);
                                    }
                                }
                            } else {
                                warn!("Live mode enabled but no executor available — check credentials");
                            }
                        }
                    }
                }
            }
        }

        // 3b. Execute signals for test portfolios
        if let Some(ref ts) = test_sessions {
            let test_signals =
                tail_risk::scan_for_testing(config.test_max_price_max, &market_data).await;

            if !test_signals.is_empty() {
                let mut ts_lock = ts.write().await;
                for tp in ts_lock.iter_mut() {
                    for signal in &test_signals {
                        if signal.current_price > tp.config.max_price {
                            continue;
                        }
                        if tp.portfolio.open_positions.values().any(|p| p.token_id == signal.token_id) {
                            continue;
                        }
                        if tp.portfolio.num_open_positions() >= config.max_open_positions {
                            continue;
                        }
                        if let Some(last) = tp.cooldowns.get(&signal.token_id) {
                            if last.elapsed().as_secs() < config.cooldown_sec {
                                continue;
                            }
                        }
                        let bet = tp.config.bet_usd.min(config.max_bet_usd);
                        if bet > tp.portfolio.balance {
                            continue;
                        }
                        // All test positions always use take-profit (we're testing TP levels)
                        if paper_engine.execute_signal(signal, bet, &mut tp.portfolio, true).is_ok() {
                            tp.cooldowns.insert(
                                signal.token_id.clone(),
                                std::time::Instant::now(),
                            );
                        }
                    }
                }
            }
        }

        // 3.5. P&L notifications — send when P&L crosses threshold
        if notify_threshold > Decimal::ZERO {
            let sessions_lock = sessions.read().await;
            for portfolio in sessions_lock.values() {
                let total_pnl = portfolio.total_value() - portfolio.initial_balance;
                let last_level = last_notified_pnl
                    .get(&portfolio.user_id)
                    .copied()
                    .unwrap_or(Decimal::ZERO);

                let current_bucket = (total_pnl / notify_threshold).floor();
                let last_bucket = (last_level / notify_threshold).floor();

                if current_bucket != last_bucket {
                    let emoji = if total_pnl > last_level { "\u{1f7e2}" } else { "\u{1f534}" };
                    let sign = if total_pnl >= Decimal::ZERO { "+" } else { "" };
                    let pnl_pct = if portfolio.initial_balance > Decimal::ZERO {
                        total_pnl / portfolio.initial_balance * dec!(100)
                    } else {
                        Decimal::ZERO
                    };

                    let msg = format!(
                        "{emoji} P&L Update: {sign}${:.2} ({sign}{:.1}%)\n\
                         Balance: ${:.2} | Positions: {}",
                        total_pnl, pnl_pct,
                        portfolio.balance, portfolio.num_open_positions(),
                    );

                    let chat_id = ChatId(portfolio.user_id);
                    if let Err(e) = bot.send_message(chat_id, &msg).await {
                        warn!("Failed to send P&L notification to user {}: {e}", portfolio.user_id);
                    }

                    last_notified_pnl.insert(portfolio.user_id, total_pnl);
                }
            }
        }

        // 4. Save sessions to DynamoDB periodically or after trades
        if traded || last_save.elapsed() >= save_interval {
            save_all_sessions(&sessions, &db).await;
            last_save = std::time::Instant::now();
        }

        // 5. Periodic cleanup
        risk_manager.cleanup_cooldowns();
        if let Some(ref ts) = test_sessions {
            let mut ts_lock = ts.write().await;
            for tp in ts_lock.iter_mut() {
                tp.cooldowns
                    .retain(|_, t| t.elapsed().as_secs() < config.cooldown_sec * 2);
            }
        }

        time::sleep(poll_interval).await;
    }
}
