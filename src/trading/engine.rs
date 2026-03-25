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
use crate::polymarket::gamma_api::GammaClient;
use crate::strategy::mean_reversion;
use crate::strategy::ml_signal::{self, MlFilter};
use crate::strategy::risk::RiskManager;
use crate::strategy::signals::{Signal, SignalSource};

use super::autotune;
use super::executor::LiveExecutor;
use super::paper::PaperTradingEngine;
use super::portfolio::Portfolio;
use super::testing::{self, SharedTestSessions};

/// Shared user sessions — maps telegram_user_id -> Portfolio
pub type SharedSessions = Arc<RwLock<HashMap<i64, Portfolio>>>;

/// Optional shared DynamoDB store
pub type SharedDb = Option<Arc<DynamoStore>>;

pub fn new_shared_sessions() -> SharedSessions {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Save all sessions (regular + test) to DynamoDB
pub async fn save_all_sessions(
    sessions: &SharedSessions,
    db: &SharedDb,
    test_sessions: &Option<SharedTestSessions>,
) {
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

    if let Some(ts) = test_sessions {
        let ts_lock = ts.read().await;
        for tp in ts_lock.iter() {
            if let Err(e) = db.save_session(&tp.portfolio).await {
                warn!("Failed to save test session {}: {e}", tp.config.name);
            }
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
    // Load ML model once at startup (if configured)
    let ml_filter: Option<MlFilter> = {
        let cfg = shared_config.read().await;
        if cfg.ml_model_path.is_empty() {
            info!("ML model path not set — ML strategy disabled");
            None
        } else {
            match MlFilter::load(&cfg.ml_model_path, cfg.ml_win_prob_threshold) {
                Ok(f) => Some(f),
                Err(e) => {
                    warn!("Failed to load ML model from '{}': {e} — ML strategy disabled", cfg.ml_model_path);
                    None
                }
            }
        }
    };
    let (poll_interval, notify_threshold) = {
        let cfg = shared_config.read().await;
        (
            Duration::from_secs(cfg.trade_poll_interval_sec),
            cfg.pnl_notify_threshold_usd,
        )
    };
    let paper_engine = PaperTradingEngine::new();
    let mut risk_manager = RiskManager::new(&*shared_config.read().await);

    // Seed cooldowns from restored open positions so we don't immediately
    // re-enter a market we already have a position in after restart.
    {
        let cooldown_sec = shared_config.read().await.cooldown_sec;
        let sessions_lock = sessions.read().await;
        for portfolio in sessions_lock.values() {
            for position in portfolio.open_positions.values() {
                let age_secs = chrono::Utc::now()
                    .signed_duration_since(position.opened_at)
                    .num_seconds()
                    .max(0) as u64;
                if age_secs < cooldown_sec {
                    let remaining = std::time::Duration::from_secs(cooldown_sec - age_secs);
                    let fake_instant = std::time::Instant::now()
                        .checked_sub(remaining)
                        .unwrap_or(std::time::Instant::now());
                    risk_manager.seed_cooldown(position.token_id.clone(), fake_instant);
                }
            }
        }
    }

    // Track last notified P&L level per user (to avoid spam)
    let mut last_notified_pnl: HashMap<i64, Decimal> = HashMap::new();

    // Circuit breaker: Some(Instant) while trading is suspended by the budget brake
    let mut brake_until: Option<std::time::Instant> = None;

    // Live order auth failure counter: disable live trading after 3 consecutive failures
    let mut live_auth_failures: u32 = 0;
    let mut live_disabled = false;

    // Balance backoff: after a balance error, skip live orders for 60s to avoid API spam
    let mut balance_err_until: Option<std::time::Instant> = None;

    // Resolution checker: query Gamma API for positions whose market has left tracked_markets
    // (resolved markets are filtered out by active=true&closed=false in fetch_expiring_markets)
    let gamma = {
        let cfg = shared_config.read().await;
        GammaClient::new(&cfg.gamma_api_url)
    };
    let mut resolution_cycle: u32 = 0;
    const RESOLUTION_CHECK_INTERVAL: u32 = 30; // check every ~30s (with 1s poll interval)

    // Save sessions every 30 seconds
    let save_interval = Duration::from_secs(30);
    let mut last_save = std::time::Instant::now();

    // MR threshold auto-tuning (independent of testing_mode; purely in-memory)
    let mut autotune_portfolios: Vec<testing::TestPortfolio> = {
        let cfg = shared_config.read().await;
        if cfg.mr_autotune_mode {
            let portfolios = autotune::generate_autotune_portfolios(&cfg);
            info!(
                "MR autotune enabled: {} threshold points in [{:.2}, {:.2}], interval={}s",
                portfolios.len(),
                cfg.mr_autotune_threshold_min,
                cfg.mr_autotune_threshold_max,
                cfg.mr_autotune_interval_sec,
            );
            portfolios
        } else {
            Vec::new()
        }
    };
    let mut last_autotune = std::time::Instant::now();

    {
        let cfg = shared_config.read().await;
        info!(
            "Trading engine started in {:?} mode. Poll interval: {}s{}",
            cfg.trading_mode,
            cfg.trade_poll_interval_sec,
            if cfg.testing_mode {
                " [TESTING MODE]"
            } else {
                ""
            },
        );
    }

    // Give the collector time to fetch initial data
    time::sleep(Duration::from_secs(5)).await;

    loop {
        // Snapshot config for this iteration (picks up any runtime strategy changes)
        let config = shared_config.read().await.clone();

        // 1. Scan for new entry signals (skip when paused or budget brake is active)
        let skip_entries = config.trading_paused
            || brake_until
                .map(|t| std::time::Instant::now() < t)
                .unwrap_or(false);

        if let Some(until) = brake_until {
            // User manually resumed via Telegram — cancel the brake early
            if !config.trading_paused {
                brake_until = None;
                info!("Budget brake cancelled by manual resume");
            } else if std::time::Instant::now() >= until {
                brake_until = None;
                shared_config.write().await.trading_paused = false;
                info!(
                    "Budget brake released after {}s — resuming trading",
                    config.budget_brake_time_sec
                );
                let sessions_lock = sessions.read().await;
                for portfolio in sessions_lock.values() {
                    let chat_id = teloxide::types::ChatId(portfolio.user_id);
                    let _ = bot
                        .send_message(
                            chat_id,
                            "\u{1f7e2} Budget brake released — trading resumed.",
                        )
                        .await;
                }
            }
        }

        let all_signals: Vec<Signal> = if skip_entries {
            Vec::new()
        } else {
            let mut sigs = Vec::new();
            // ML-filtered MR strategy (primary): runs when budget is not 100% plain-MR
            if config.mean_reversion_budget_pct < dec!(1.0) {
                if let Some(ref filter) = ml_filter {
                    sigs.extend(
                        ml_signal::scan_ml_filtered(&config, &market_data, filter).await,
                    );
                }
            }
            // Plain MR (plan B): runs when budget allocation allows it
            if config.mean_reversion_budget_pct > Decimal::ZERO {
                sigs.extend(
                    mean_reversion::scan_mean_reversion(&config, &market_data).await,
                );
            }
            sigs
        };

        if !all_signals.is_empty() {
            debug!("Found {} signals this cycle", all_signals.len());
        }

        // 2. Build price map and update all portfolios
        let mut price_map: HashMap<String, Decimal> = {
            let data = market_data.read().await;
            let mut m = HashMap::new();
            for market in &data.tracked_markets {
                for outcome in &market.outcomes {
                    m.insert(outcome.token_id.clone(), outcome.price);
                }
            }
            m
        };

        // 2a. Resolution check: for open positions whose token has left tracked_markets,
        // periodically query Gamma API for final prices (resolved markets have closed=true).
        resolution_cycle += 1;
        if resolution_cycle >= RESOLUTION_CHECK_INTERVAL {
            resolution_cycle = 0;
            let stale_condition_ids: std::collections::HashSet<String> = {
                let sessions_lock = sessions.read().await;
                sessions_lock
                    .values()
                    .flat_map(|p| {
                        p.open_positions.values().filter_map(|pos| {
                            if !price_map.contains_key(&pos.token_id) {
                                Some(pos.condition_id.clone())
                            } else {
                                None
                            }
                        })
                    })
                    .collect()
            };
            if !stale_condition_ids.is_empty() {
                info!(
                    "Checking resolution prices for {} market(s) no longer in tracking window",
                    stale_condition_ids.len()
                );
                let resolved = gamma.fetch_prices_for_stale(&stale_condition_ids).await;
                if resolved.is_empty() {
                    info!("Resolution fetch: no closed markets found for {} condition_ids", stale_condition_ids.len());
                }
                for (cid, entries) in &resolved {
                    for (tid, price) in entries {
                        info!("Resolution: cid={cid} token={tid} price={price:.4}");
                        price_map.entry(tid.clone()).or_insert(*price);
                    }
                }
            }
        }

        {
            let clob_balance = if config.trading_mode == TradingMode::Live {
                if let Some(ref executor) = live_executor {
                    executor.clob_balance().await
                } else {
                    None
                }
            } else {
                None
            };

            let mut sessions_lock = sessions.write().await;
            for portfolio in sessions_lock.values_mut() {
                portfolio.update_prices(&price_map);
                if let Some(bal) = clob_balance {
                    portfolio.live_clob_balance = Some(bal);
                    // In live mode, keep portfolio.balance in sync with actual CLOB balance
                    portfolio.balance = bal;
                }
            }
        }

        // 2b. Update prices in test portfolios (paper mode only — skip in live mode)
        if config.trading_mode != TradingMode::Live {
            if let Some(ref ts) = test_sessions {
                let mut ts_lock = ts.write().await;
                for tp in ts_lock.iter_mut() {
                    tp.portfolio.update_prices(&price_map);
                }
            }
            for tp in autotune_portfolios.iter_mut() {
                tp.portfolio.update_prices(&price_map);
            }
        }

        let mut traded = false;

        // 2.5. Auto-exit: check resolution on all open positions.
        // Phase 1: close locally (write lock, no async), collect live sells needed.
        // Phase 2: submit sell orders to CLOB after releasing the lock.
        let live_sells: Vec<(String, Decimal, Decimal)> = Vec::new(); // (token_id, qty, price)
        {
            let mut sessions_lock = sessions.write().await;
            for portfolio in sessions_lock.values_mut() {
                let position_ids: Vec<String> = portfolio.open_positions.keys().cloned().collect();
                for pos_id in position_ids {
                    let current_price = portfolio.open_positions[&pos_id].current_price;
                    if current_price >= dec!(0.97) {
                        match portfolio.close_position(&pos_id, current_price, "resolved_win") {
                            Ok(trade) => {
                                traded = true;
                                info!(
                                    "RESOLVED WIN for user {}: {} | P&L: ${:.2} ({:.1}%)",
                                    portfolio.user_id,
                                    trade.outcome_name,
                                    trade.realized_pnl,
                                    trade.realized_pnl_pct
                                );
                            }
                            Err(e) => warn!("Failed to close resolved win: {e}"),
                        }
                    } else if current_price <= dec!(0.03) {
                        match portfolio.close_position(&pos_id, current_price, "resolved_loss") {
                            Ok(trade) => {
                                traded = true;
                                info!(
                                    "RESOLVED LOSS for user {}: {} | P&L: ${:.2} ({:.1}%)",
                                    portfolio.user_id,
                                    trade.outcome_name,
                                    trade.realized_pnl,
                                    trade.realized_pnl_pct
                                );
                            }
                            Err(e) => warn!("Failed to close resolved loss: {e}"),
                        }
                    }
                }
            }
        }

        // Phase 2: submit sell orders to CLOB for any TP/SL exits in live mode
        if config.trading_mode == TradingMode::Live && !live_sells.is_empty() {
            if let Some(ref executor) = live_executor {
                for (token_id, qty, price) in &live_sells {
                    match executor.sell_position(token_id, *qty, *price).await {
                        Ok(order_id) => {
                            info!("Live sell submitted: token={token_id} qty={qty} price={price} order={order_id}");
                            balance_err_until = None; // sells going through, balance will recover
                        }
                        Err(e) => warn!("Live sell failed for {token_id}: {e}"),
                    }
                }
            }
        }

        // 2.6. Auto-exit for test portfolios
        if let Some(ref ts) = test_sessions {
            let mut ts_lock = ts.write().await;
            for tp in ts_lock.iter_mut() {
                let position_ids: Vec<String> =
                    tp.portfolio.open_positions.keys().cloned().collect();
                for pos_id in position_ids {
                    let current_price = tp.portfolio.open_positions[&pos_id].current_price;
                    if current_price >= dec!(0.97) {
                        let _ = tp
                            .portfolio
                            .close_position(&pos_id, current_price, "resolved_win");
                    } else if current_price <= dec!(0.03) {
                        let _ =
                            tp.portfolio
                                .close_position(&pos_id, current_price, "resolved_loss");
                    }
                }
            }
        }

        // 2.7. Auto-exit for autotune portfolios
        for tp in autotune_portfolios.iter_mut() {
            let position_ids: Vec<String> = tp.portfolio.open_positions.keys().cloned().collect();
            for pos_id in position_ids {
                let current_price = tp.portfolio.open_positions[&pos_id].current_price;
                if current_price >= dec!(0.97) {
                    let _ = tp.portfolio.close_position(&pos_id, current_price, "resolved_win");
                } else if current_price <= dec!(0.03) {
                    let _ = tp.portfolio.close_position(&pos_id, current_price, "resolved_loss");
                }
            }
        }

        // 3. Execute signals for each active user session (skipped when paused/braked)
        if !skip_entries && !all_signals.is_empty() {
            risk_manager.update_from_config(&config);
            let mut sessions_lock = sessions.write().await;

            for portfolio in sessions_lock.values_mut() {
                for signal in &all_signals {
                    if portfolio
                        .open_positions
                        .values()
                        .any(|p| p.token_id == signal.token_id)
                    {
                        continue;
                    }

                    let open_pos = portfolio.num_open_positions();
                    let balance = match config.trading_mode {
                        TradingMode::Live => {
                            portfolio.live_clob_balance.unwrap_or(portfolio.balance)
                        }
                        TradingMode::Paper => portfolio.balance,
                    };

                    let amount = match risk_manager.evaluate_mr(signal, balance, open_pos) {
                        Some(a) => a,
                        None => continue,
                    };

                    // Budget enforcement: MR must always have its reserved share of initial_balance.
                    // ML is capped at (1 - mr_pct) * initial_balance to prevent crowding out MR.
                    let mr_pct = config.mean_reversion_budget_pct;
                    if mr_pct > Decimal::ZERO && mr_pct < dec!(1.0) {
                        let ml_invested: Decimal = portfolio
                            .open_positions
                            .values()
                            .filter(|p| p.source == SignalSource::MlFiltered)
                            .map(|p| p.cost_basis)
                            .sum();
                        let mr_invested: Decimal = portfolio
                            .open_positions
                            .values()
                            .filter(|p| p.source == SignalSource::MeanReversion)
                            .map(|p| p.cost_basis)
                            .sum();
                        let reference = portfolio.initial_balance;
                        match signal.source {
                            SignalSource::MlFiltered => {
                                let ml_cap = reference * (dec!(1.0) - mr_pct);
                                if ml_invested + amount > ml_cap {
                                    debug!(
                                        "Budget: ML cap ${ml_cap:.2} reached (invested ${ml_invested:.2}), skipping"
                                    );
                                    continue;
                                }
                            }
                            SignalSource::MeanReversion => {
                                let mr_cap = reference * mr_pct;
                                if mr_invested + amount > mr_cap {
                                    debug!(
                                        "Budget: MR cap ${mr_cap:.2} reached (invested ${mr_invested:.2}), skipping"
                                    );
                                    continue;
                                }
                            }
                        }
                    }

                    // All positions hold to market resolution (no TP/SL)
                    let use_tp = false;

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
                            if live_disabled {
                                // Auth failed too many times — skip live orders silently
                            } else if balance_err_until
                                .map(|t| std::time::Instant::now() < t)
                                .unwrap_or(false)
                            {
                                // Recently got a balance error — skip API call until backoff expires
                                debug!(
                                    "Skipping live order (balance backoff active): {}",
                                    signal.token_id
                                );
                            } else if let Some(ref executor) = live_executor {
                                match executor.execute_signal(signal, amount).await {
                                    Ok(order_id) => {
                                        live_auth_failures = 0;
                                        balance_err_until = None;
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
                                        debug!(
                                            "Live order failed for user {}: {e}",
                                            portfolio.user_id
                                        );
                                        let err_str = e.to_string();
                                        let is_auth_error = err_str.contains("401")
                                            || err_str.contains("Unauthorized")
                                            || err_str.contains("Invalid api key");
                                        let is_balance_err = super::executor::is_balance_error(&e);
                                        if is_auth_error {
                                            live_auth_failures += 1;
                                        }
                                        if live_auth_failures >= 3 {
                                            live_disabled = true;
                                            let msg = format!(
                                                "\u{26a0}\u{fe0f} Live trading disabled after {} consecutive auth failures.\n\
                                                 Last error: {e}\n\n\
                                                 Check your WALLET_PRIVATE_KEY and restart the bot.",
                                                live_auth_failures
                                            );
                                            warn!("{msg}");
                                            let chat_id = ChatId(portfolio.user_id);
                                            let _ = bot.send_message(chat_id, &msg).await;
                                        } else if is_balance_err {
                                            // Back off for 60s — free balance may be locked in resting orders
                                            balance_err_until = Some(
                                                std::time::Instant::now() + Duration::from_secs(60),
                                            );
                                            debug!("Insufficient balance, backing off 60s");
                                        }
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

        // 3b. Execute signals for test portfolios (paper mode only; skipped when paused/braked)
        if !skip_entries && config.trading_mode != TradingMode::Live {
            if let Some(ref ts) = test_sessions {
                // MR signals: scanned with minimum threshold (each portfolio filters further)
                let min_mr_threshold = config
                    .test_mr_threshold_min
                    .to_string()
                    .parse::<f64>()
                    .unwrap_or(0.10);
                let test_mr_signals = mean_reversion::scan_mr_for_testing(
                    min_mr_threshold,
                    &market_data,
                    config.min_liquidity_usd,
                    config.mr_market_expiry_window_hours,
                )
                .await;

                // ML signals: scanned with minimum ML threshold (each ML portfolio filters further)
                let test_ml_signals: Vec<Signal> = if let Some(ref filter) = ml_filter {
                    ml_signal::scan_ml_for_testing(
                        config.test_ml_threshold_min,
                        &config,
                        &market_data,
                        filter,
                    )
                    .await
                } else {
                    Vec::new()
                };

                let mut ts_lock = ts.write().await;
                for tp in ts_lock.iter_mut() {
                    let signals: &[Signal] = match tp.config.strategy {
                        testing::TestStrategy::Mr => &test_mr_signals,
                        testing::TestStrategy::Ml => &test_ml_signals,
                    };
                    for signal in signals {
                        // Portfolio-specific threshold filter
                        let passes = match tp.config.strategy {
                            testing::TestStrategy::Mr => {
                                signal.pct_change.abs() >= tp.config.threshold
                            }
                            testing::TestStrategy::Ml => signal.win_prob >= tp.config.threshold,
                        };
                        if !passes {
                            continue;
                        }

                        if tp
                            .portfolio
                            .open_positions
                            .values()
                            .any(|p| p.token_id == signal.token_id)
                        {
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
                        if paper_engine
                            .execute_signal(signal, bet, &mut tp.portfolio, false)
                            .is_ok()
                        {
                            tp.cooldowns
                                .insert(signal.token_id.clone(), std::time::Instant::now());
                        }
                    }
                }
            }
        } // end !skip_entries for test portfolios

        // 3c. Execute MR signals for autotune portfolios (paper only, not paused by trading pause)
        if !autotune_portfolios.is_empty() && config.trading_mode != TradingMode::Live {
            let min_thr = config
                .mr_autotune_threshold_min
                .to_string()
                .parse::<f64>()
                .unwrap_or(0.10);
            let autotune_signals = mean_reversion::scan_mr_for_testing(
                min_thr,
                &market_data,
                config.min_liquidity_usd,
                config.mr_market_expiry_window_hours,
            )
            .await;

            for tp in autotune_portfolios.iter_mut() {
                for signal in &autotune_signals {
                    if signal.pct_change.abs() < tp.config.threshold {
                        continue;
                    }
                    if tp
                        .portfolio
                        .open_positions
                        .values()
                        .any(|p| p.token_id == signal.token_id)
                    {
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
                    if paper_engine
                        .execute_signal(signal, bet, &mut tp.portfolio, false)
                        .is_ok()
                    {
                        tp.cooldowns
                            .insert(signal.token_id.clone(), std::time::Instant::now());
                    }
                }
            }
        }

        // 3.5. Budget brake — check after exits/entries each cycle
        // Skip entirely if pause duration is 0 (would cause instant release → spam loop)
        if brake_until.is_none()
            && config.budget_brake_pct > Decimal::ZERO
            && config.budget_brake_time_sec > 0
        {
            let sessions_lock = sessions.read().await;
            for portfolio in sessions_lock.values() {
                if portfolio.initial_balance == Decimal::ZERO {
                    continue;
                }
                let loss_pct = (portfolio.initial_balance - portfolio.total_value())
                    / portfolio.initial_balance
                    * dec!(100);
                if loss_pct >= config.budget_brake_pct {
                    let until = std::time::Instant::now()
                        + Duration::from_secs(config.budget_brake_time_sec);
                    brake_until = Some(until);
                    shared_config.write().await.trading_paused = true;
                    warn!(
                        "BUDGET BRAKE triggered for user {}: -{:.1}% loss (threshold -{:.1}%). \
                         Pausing trading for {}s.",
                        portfolio.user_id,
                        loss_pct,
                        config.budget_brake_pct,
                        config.budget_brake_time_sec
                    );
                    let mins = config.budget_brake_time_sec / 60;
                    let msg = format!(
                        "\u{1f6d1} Budget brake triggered: -{:.1}% loss.\n\
                         Trading paused for {} min. Will resume automatically.",
                        loss_pct, mins
                    );
                    let chat_id = teloxide::types::ChatId(portfolio.user_id);
                    let _ = bot.send_message(chat_id, msg).await;
                    break;
                }
            }
        }

        // 3.6. P&L notifications — send when P&L crosses threshold
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
                    let emoji = if total_pnl > last_level {
                        "\u{1f7e2}"
                    } else {
                        "\u{1f534}"
                    };
                    let sign = if total_pnl >= Decimal::ZERO { "+" } else { "" };
                    let msg = format!(
                        "{emoji} P&L Update: {sign}${:.2}\n\
                         Balance: ${:.2} | Positions: {}",
                        total_pnl,
                        portfolio.balance,
                        portfolio.num_open_positions(),
                    );

                    let chat_id = ChatId(portfolio.user_id);
                    if let Err(e) = bot.send_message(chat_id, &msg).await {
                        warn!(
                            "Failed to send P&L notification to user {}: {e}",
                            portfolio.user_id
                        );
                    }

                    last_notified_pnl.insert(portfolio.user_id, total_pnl);
                }
            }
        }

        // 4. Save sessions to DynamoDB periodically or after trades
        if traded || last_save.elapsed() >= save_interval {
            save_all_sessions(&sessions, &db, &test_sessions).await;
            last_save = std::time::Instant::now();
        }

        // 4.5. MR autotune: evaluate last-hour performance and update threshold
        if config.mr_autotune_mode
            && !autotune_portfolios.is_empty()
            && last_autotune.elapsed().as_secs() >= config.mr_autotune_interval_sec
        {
            last_autotune = std::time::Instant::now();
            match autotune::find_best_mr_threshold(&autotune_portfolios, 1) {
                Some((best_thr, roi_pct)) => {
                    let current_thr = config.mean_reversion_threshold;
                    shared_config.write().await.mean_reversion_threshold = best_thr;
                    if best_thr != current_thr {
                        info!(
                            "MR autotune: threshold updated {:.3} → {:.3} (last-hour ROI: {:.1}%)",
                            current_thr, best_thr, roi_pct,
                        );
                    } else {
                        info!(
                            "MR autotune: threshold {:.3} confirmed (last-hour ROI: {:.1}%)",
                            best_thr, roi_pct,
                        );
                    }
                }
                None => {
                    debug!("MR autotune: no recent trades to evaluate — threshold unchanged");
                }
            }
            // Refill balances so every threshold has equal capital next interval
            autotune::refill_balances(&mut autotune_portfolios);
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
