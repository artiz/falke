use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{self, Duration};
use tracing::{info, warn};

use crate::config::{Config, TradingMode};
use crate::market_data::collector::SharedMarketData;
use crate::strategy::{arbitrage, momentum};
use crate::strategy::risk::RiskManager;
use crate::strategy::signals::{Signal, SignalSource};

use super::paper::PaperTradingEngine;
use super::portfolio::Portfolio;

/// Shared user sessions — maps telegram_user_id -> Portfolio
pub type SharedSessions = Arc<RwLock<HashMap<i64, Portfolio>>>;

pub fn new_shared_sessions() -> SharedSessions {
    Arc::new(RwLock::new(HashMap::new()))
}

/// The main trading engine loop.
///
/// Runs every poll interval:
/// 1. Scans for arbitrage opportunities
/// 2. Scans for momentum signals
/// 3. Evaluates signals through risk manager
/// 4. Executes trades (paper or live) for each active user session
pub async fn run_engine(
    config: Config,
    market_data: SharedMarketData,
    sessions: SharedSessions,
) {
    let poll_interval = Duration::from_secs(config.momentum_poll_interval_sec);
    let paper_engine = PaperTradingEngine::new();
    let mut risk_manager = RiskManager::new(&config);

    info!(
        "Trading engine started in {:?} mode. Poll interval: {}s",
        config.trading_mode, config.momentum_poll_interval_sec
    );

    // Give the collector time to fetch initial data
    time::sleep(Duration::from_secs(5)).await;

    loop {
        // 1. Scan for signals
        let arb_signals = arbitrage::scan_arbitrage(&config, &market_data).await;
        let mom_signals = momentum::scan_momentum(&config, &market_data).await;

        let all_signals: Vec<Signal> = arb_signals
            .into_iter()
            .chain(mom_signals.into_iter())
            .collect();

        if !all_signals.is_empty() {
            info!("Found {} signals this cycle", all_signals.len());
        }

        // 2. Update prices in all portfolios
        {
            let data = market_data.read().await;
            let mut price_map: HashMap<String, rust_decimal::Decimal> = HashMap::new();
            for market in &data.tracked_markets {
                for outcome in &market.outcomes {
                    price_map.insert(outcome.token_id.clone(), outcome.price);
                }
            }

            let mut sessions_lock = sessions.write().await;
            for portfolio in sessions_lock.values_mut() {
                portfolio.update_prices(&price_map);
            }
        }

        // 3. Execute signals for each active user session
        if !all_signals.is_empty() {
            let mut sessions_lock = sessions.write().await;

            for portfolio in sessions_lock.values_mut() {
                for signal in &all_signals {
                    let open_pos = portfolio.num_open_positions();
                    let balance = portfolio.balance;

                    // Risk check
                    let amount = match risk_manager.evaluate(signal, balance, open_pos) {
                        Some(a) => a,
                        None => continue,
                    };

                    // Execute based on trading mode
                    match config.trading_mode {
                        TradingMode::Paper => {
                            let result = match signal.source {
                                SignalSource::Arbitrage => {
                                    paper_engine
                                        .execute_arb_signal(signal, amount, portfolio)
                                        .map(|ids| ids.join(", "))
                                }
                                SignalSource::Momentum => {
                                    paper_engine.execute_signal(signal, amount, portfolio)
                                }
                            };

                            match result {
                                Ok(id) => {
                                    risk_manager.record_trade(&signal.condition_id);
                                    info!(
                                        "Paper trade executed for user {}: {}",
                                        portfolio.user_id, id
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "Paper trade failed for user {}: {e}",
                                        portfolio.user_id
                                    );
                                }
                            }
                        }
                        TradingMode::Live => {
                            // Live trading will be implemented with the executor
                            warn!("Live trading not yet implemented, skipping signal");
                        }
                    }
                }
            }
        }

        // 4. Periodic cleanup
        risk_manager.cleanup_cooldowns();

        time::sleep(poll_interval).await;
    }
}
