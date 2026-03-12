mod config;
mod db;
mod error;
mod market_data;
mod polymarket;
mod strategy;
mod telegram;
mod trading;
mod utils;

use tracing::info;
use tracing_subscriber::EnvFilter;

use config::Config;
use market_data::collector;
use trading::engine;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
        "Strategy: {}% Arb / {}% Momentum",
        config.arb_budget_pct * rust_decimal_macros::dec!(100),
        config.momentum_budget_pct * rust_decimal_macros::dec!(100),
    );
    info!("Market expiry window: {} days", config.market_expiry_window_days);
    info!("Monitoring poll interval: {}s", config.momentum_poll_interval_sec);

    // Initialize shared state
    let market_data = collector::new_shared_market_data(&config);
    let sessions = engine::new_shared_sessions();

    // Spawn the market data collector
    let collector_config = config.clone();
    let collector_data = market_data.clone();
    tokio::spawn(async move {
        collector::run_collector(collector_config, collector_data).await;
    });

    // Spawn the trading engine
    let engine_config = config.clone();
    let engine_data = market_data.clone();
    let engine_sessions = sessions.clone();
    tokio::spawn(async move {
        engine::run_engine(engine_config, engine_data, engine_sessions).await;
    });

    // Run the Telegram bot (this blocks)
    telegram::bot::run_bot(config, sessions, market_data).await;

    Ok(())
}
