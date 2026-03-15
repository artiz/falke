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

use config::{Config, SharedConfig, TradingMode};
use db::dynamo::DynamoStore;
use market_data::collector;
use polymarket::{auth, clob_api::ClobClient};
use trading::{engine, executor::LiveExecutor};

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

    info!("Starting Falke Trading Bot...");

    // Load configuration
    let config = Config::from_env()?;

    info!("Trading mode: {:?}", config.trading_mode);
    info!("Paper balance: ${}", config.paper_balance);
    info!(
        "Strategy: {}% Arb / {}% Mom / {}% MR / {}% Tail",
        config.arb_budget_pct * rust_decimal_macros::dec!(100),
        config.momentum_budget_pct * rust_decimal_macros::dec!(100),
        config.mean_reversion_budget_pct * rust_decimal_macros::dec!(100),
        config.tail_risk_budget_pct * rust_decimal_macros::dec!(100),
    );
    info!(
        "Risk: TP={}% / SL={}% | Max bet=${} | Max positions={}",
        config.take_profit_pct, config.stop_loss_pct, config.max_bet_usd, config.max_open_positions,
    );
    info!(
        "Market expiry window: {} days",
        config.market_expiry_window_days
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

    // Initialize shared state
    let market_data = collector::new_shared_market_data(&*shared_config.read().await);
    let sessions = engine::new_shared_sessions();

    // Restore sessions from DynamoDB
    if let Some(ref db) = db {
        match db.load_all_sessions().await {
            Ok(restored) => {
                if !restored.is_empty() {
                    info!("Restored {} sessions from DynamoDB", restored.len());
                    let mut s = sessions.write().await;
                    *s = restored;
                }
            }
            Err(e) => {
                warn!("Failed to restore sessions: {e}");
            }
        }
    }

    // Initialize live executor if credentials are available
    let live_executor = {
        let cfg = shared_config.read().await;
        if cfg.trading_mode == TradingMode::Live {
            match auth::resolve_relayer_credentials(&cfg) {
                Some(creds) => {
                    let clob = ClobClient::new(&cfg.clob_api_url).with_relayer_credentials(creds);
                    info!("Live executor initialized with Relayer API credentials");
                    Some(Arc::new(LiveExecutor::new(clob)))
                }
                None => {
                    warn!("Live mode enabled but RELAYER_API_KEY/RELAYER_API_KEY_ADDRESS not set. Falling back to paper mode.");
                    None
                }
            }
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

    // Create Bot instance (shared between engine and telegram handler)
    let bot = teloxide::Bot::new(&shared_config.read().await.telegram_bot_token);

    // Spawn the trading engine
    let engine_cfg = shared_config.clone();
    let engine_data = market_data.clone();
    let engine_sessions = sessions.clone();
    let engine_db = db.clone();
    let engine_bot = bot.clone();
    tokio::spawn(async move {
        engine::run_engine(
            engine_cfg,
            engine_data,
            engine_sessions,
            engine_db,
            engine_bot,
            live_executor,
        )
        .await;
    });

    // Run the Telegram bot (this blocks)
    telegram::bot::run_bot(shared_config, sessions, market_data, db, bot).await;

    Ok(())
}
