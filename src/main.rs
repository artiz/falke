mod config;
mod db;
mod error;
mod market_data;
mod polymarket;
mod strategy;
mod telegram;
mod trading;

#[allow(dead_code)]
mod utils;

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

fn parse_reset_flag() -> bool {
    std::env::args().any(|a| a == "--reset")
}

use config::{Config, SharedConfig, TradingMode};
use db::dynamo::DynamoStore;
use market_data::collector;
use polymarket::auth;
use trading::{engine, executor::LiveExecutor, reconcile, testing};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env before tracing so RUST_LOG is available
    let _ = dotenvy::dotenv();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("falke=info")),
        )
        .init();

    let reset_session = parse_reset_flag();
    if reset_session {
        info!("--reset flag detected: current session will be cleared on startup");
    }

    info!("Starting Falke Trading Bot...");

    // Load configuration
    let config = Config::from_env()?;

    info!("Trading mode: {:?}", config.trading_mode);
    info!("Paper balance: ${}", config.paper_balance);
    info!(
        "Strategy: Tail Risk | Max bet=${} | Max positions={}",
        config.max_bet_usd, config.max_open_positions,
    );
    info!(
        "Market expiry window: {}h",
        config.market_expiry_window_hours
    );
    info!(
        "Monitoring poll interval: {}s",
        config.trade_poll_interval_sec
    );

    let shared_config: SharedConfig = Arc::new(RwLock::new(config));

    // Initialize DynamoDB
    let db = match DynamoStore::new(&*shared_config.read().await).await {
        Ok(store) => {
            info!("DynamoDB initialized");
            Some(Arc::new(store))
        }
        Err(e) => {
            warn!("DynamoDB initialization failed: {e}. Running without persistence.");
            None
        }
    };

    // Load persisted settings from DB and merge on top of env config (DB wins)
    if let Some(ref db) = db {
        match db.load_global_settings().await {
            Ok(settings) => {
                // Detect trading mode change and clear stale sessions
                let current_mode = match shared_config.read().await.trading_mode {
                    config::TradingMode::Live => "live",
                    config::TradingMode::Paper => "paper",
                };
                if let Some(ref stored_mode) = settings.trading_mode {
                    if stored_mode.as_str() != current_mode {
                        warn!(
                            "Trading mode changed: {} → {}. Clearing all sessions.",
                            stored_mode, current_mode
                        );
                        match db.clear_all_sessions().await {
                            Ok(()) => info!("Sessions cleared after mode switch."),
                            Err(e) => warn!("Failed to clear sessions after mode switch: {e}"),
                        }
                    }
                }

                shared_config.write().await.apply_db_settings(&settings);
                info!(
                    "DB settings applied: paused={}, tp={}, bet={}, max_price={}, window={}",
                    settings.paused,
                    settings
                        .tail_risk_take_profit_pct
                        .map_or("env".into(), |v| v.to_string()),
                    settings
                        .tail_risk_bet_usd
                        .map_or("env".into(), |v| v.to_string()),
                    settings
                        .tail_risk_max_price
                        .map_or("env".into(), |v| v.to_string()),
                    settings
                        .market_expiry_window_hours
                        .map_or("env".into(), |v| v.to_string()),
                );

                // Persist the current mode so next startup can detect changes
                let mut updated = settings.clone();
                updated.trading_mode = Some(current_mode.to_string());
                if let Err(e) = db.save_global_settings(&updated).await {
                    warn!("Failed to persist trading mode to DB settings: {e}");
                }
            }
            Err(e) => warn!("Could not load DB settings ({e}), using env defaults."),
        }
    }

    // Initialize shared state
    let market_data = collector::new_shared_market_data(&*shared_config.read().await);
    let sessions = engine::new_shared_sessions();

    // Restore sessions from DynamoDB
    if let Some(ref db) = db {
        match db.load_all_sessions().await {
            Ok(existing) if !existing.is_empty() => {
                let initial_balance = shared_config.read().await.paper_balance;
                let mut s = sessions.write().await;
                if reset_session {
                    // Keep users registered but wipe portfolio state
                    for (user_id, _) in &existing {
                        let fresh = trading::portfolio::Portfolio::new(*user_id, initial_balance);
                        if let Err(e) = db.save_session(&fresh).await {
                            warn!("--reset: failed to persist fresh session for {user_id}: {e}");
                        }
                        s.insert(*user_id, fresh);
                    }
                    info!(
                        "--reset: reset {} session(s) to fresh portfolio (users stay registered)",
                        existing.len()
                    );
                } else {
                    info!("Restored {} sessions from DynamoDB", existing.len());
                    *s = existing;
                }
            }
            Ok(_) => {
                if reset_session {
                    info!("--reset: no existing sessions to reset");
                }
            }
            Err(e) => warn!("Failed to restore sessions: {e}"),
        }
    }

    // Create Bot instance early so reconcile can send Telegram notifications
    let bot = teloxide::Bot::new(&shared_config.read().await.telegram_bot_token);

    // Initialize live executor (SDK auth) and reconcile restored sessions
    let live_executor = {
        let cfg = shared_config.read().await;
        if cfg.trading_mode == TradingMode::Live {
            match auth::authenticate_live(&cfg).await {
                Ok((sdk_client, signer)) => {
                    use polymarket::clob_api::ClobClient;
                    let clob = ClobClient::new(
                        sdk_client,
                        signer,
                        cfg.polygon_rpc_url.clone(),
                        cfg.process_usdc_allowances,
                    );

                    // Ensure USDC allowance is set for the CTF Exchange contract
                    match clob.ensure_allowance().await {
                        Ok(()) => info!("USDC allowance confirmed"),
                        Err(e) => warn!("Could not set USDC allowance: {e}. Orders may fail."),
                    }

                    // In live mode, always sync balance and initial_balance from CLOB.
                    // This ensures P&L is calculated against the real starting balance,
                    // not a stale paper-mode value.
                    if let Some(clob_bal) = clob.balance_usdc().await {
                        let mut s = sessions.write().await;
                        for portfolio in s.values_mut() {
                            portfolio.balance = clob_bal;
                            portfolio.initial_balance = clob_bal;
                        }
                        info!(
                            "Live mode: portfolio balance synced from CLOB: ${:.2}",
                            clob_bal
                        );
                    }

                    // Reconcile open positions against CLOB before starting the engine
                    reconcile::reconcile_live_positions(&clob, &sessions, &bot).await;

                    info!("Live executor ready");
                    Some(Arc::new(LiveExecutor::new(clob)))
                }
                Err(e) => {
                    warn!("Live mode: authentication failed ({e}). Falling back to paper mode.");
                    None
                }
            }
        } else {
            None
        }
    };

    // Initialize test portfolios if testing mode is enabled
    let test_sessions = {
        let cfg = shared_config.read().await;
        if cfg.testing_mode {
            let ts = testing::new_shared_test_sessions();
            let portfolios = testing::generate_test_portfolios(&cfg);
            info!("Testing mode: {} strategies generated", portfolios.len());
            *ts.write().await = portfolios;
            Some(ts)
        } else {
            None
        }
    };

    // Spawn the market data collector
    let collector_config = shared_config.read().await.clone();
    let collector_data = market_data.clone();
    tokio::spawn(async move {
        collector::run_collector(collector_config, collector_data).await;
    });

    // Spawn the trading engine
    let engine_cfg = shared_config.clone();
    let engine_data = market_data.clone();
    let engine_sessions = sessions.clone();
    let engine_db = db.clone();
    let engine_bot = bot.clone();
    let engine_test = test_sessions.clone();
    let engine_executor = live_executor.clone();
    tokio::spawn(async move {
        engine::run_engine(
            engine_cfg,
            engine_data,
            engine_sessions,
            engine_db,
            engine_bot,
            engine_executor,
            engine_test,
        )
        .await;
    });

    // Run the Telegram bot (this blocks)
    telegram::bot::run_bot(
        shared_config,
        sessions,
        market_data,
        db,
        bot,
        live_executor,
        test_sessions,
    )
    .await;

    Ok(())
}
