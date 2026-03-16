use rust_decimal::Decimal;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{self, Duration};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::polymarket::gamma_api::GammaClient;
use crate::polymarket::types::TrackedMarket;

use super::price_store::PriceStore;

/// Shared state for market data accessible by strategies and the trading engine
pub struct MarketData {
    pub tracked_markets: Vec<TrackedMarket>,
    pub price_store: PriceStore,
}

pub type SharedMarketData = Arc<RwLock<MarketData>>;

pub fn new_shared_market_data(_config: &Config) -> SharedMarketData {
    Arc::new(RwLock::new(MarketData {
        tracked_markets: Vec::new(),
        price_store: PriceStore::new(
            1000, // max history per token
            300,  // window_sec (5 min)
            5,    // min data points for derivative
        ),
    }))
}

/// The market data collector runs as a background task.
/// It periodically fetches markets and their prices, updating the shared state.
pub async fn run_collector(config: Config, market_data: SharedMarketData) {
    let gamma = GammaClient::new(&config.gamma_api_url);
    let poll_interval = Duration::from_secs(config.trade_poll_interval_sec);

    // Fetch market list less frequently (every 30 x trade_poll_interval_sec)
    let market_refresh_interval = Duration::from_secs(config.trade_poll_interval_sec  * 30);
    let mut last_market_refresh = std::time::Instant::now() - market_refresh_interval;

    info!(
        "Market data collector started. Poll interval: {}s, Expiry window: {} days",
        config.trade_poll_interval_sec, config.market_expiry_window_days
    );

    loop {
        // Refresh market list periodically
        let now = std::time::Instant::now();
        if now.duration_since(last_market_refresh) >= market_refresh_interval {
            match gamma
                .fetch_expiring_markets(config.market_expiry_window_days)
                .await
            {
                Ok(gamma_markets) => {
                    let tracked: Vec<TrackedMarket> = gamma_markets
                        .iter()
                        .filter_map(|m| GammaClient::to_tracked_market(m))
                        .filter(|m| m.liquidity >= config.min_liquidity_usd)
                        .collect();

                    info!("Refreshed market list: {} markets tracked", tracked.len());

                    let mut data = market_data.write().await;

                    // Collect active token IDs for cleanup
                    let active_tokens: HashSet<String> = tracked
                        .iter()
                        .flat_map(|m| m.outcomes.iter().map(|o| o.token_id.clone()))
                        .collect();

                    data.tracked_markets = tracked;
                    data.price_store.retain_tokens(&active_tokens);
                }
                Err(e) => {
                    error!("Failed to fetch markets: {e}");
                }
            }
            last_market_refresh = now;
        }

        // Record current prices for all tracked outcomes into the price store
        {
            let mut data = market_data.write().await;
            let markets = data.tracked_markets.clone();

            for market in &markets {
                for outcome in &market.outcomes {
                    data.price_store.add_price(&outcome.token_id, outcome.price);
                }
            }

            let stats = data.price_store.stats();
            debug!(
                "Price store: {} tokens, {} points, ~{} KB",
                stats.num_tokens_tracked,
                stats.total_data_points,
                stats.approx_memory_bytes / 1024
            );
        }

        // Fetch latest prices and update tracked markets
        match gamma
            .fetch_expiring_markets(config.market_expiry_window_days)
            .await
        {
            Ok(gamma_markets) => {
                let mut data = market_data.write().await;
                let mut price_updates: Vec<(String, Decimal)> = Vec::new();

                for gm in &gamma_markets {
                    if let Some(tracked) = data
                        .tracked_markets
                        .iter_mut()
                        .find(|m| m.condition_id == gm.condition_id)
                    {
                        // Parse the updated prices from the Gamma API response
                        let new_prices = gm.parsed_prices();
                        let new_token_ids = gm.parsed_token_ids();

                        for (i, outcome) in tracked.outcomes.iter_mut().enumerate() {
                            // Match by token ID or by index
                            let new_price = if !new_token_ids.is_empty() {
                                new_token_ids
                                    .iter()
                                    .position(|id| *id == outcome.token_id)
                                    .and_then(|idx| new_prices.get(idx))
                                    .copied()
                            } else {
                                new_prices.get(i).copied()
                            };

                            if let Some(price_f64) = new_price {
                                let price = Decimal::from_str(&format!("{price_f64:.6}"))
                                    .unwrap_or(Decimal::ZERO);
                                outcome.price = price;
                                price_updates.push((outcome.token_id.clone(), price));
                            }
                        }

                        tracked.last_updated = chrono::Utc::now();
                    }
                }

                // Apply price updates to the store
                for (token_id, price) in price_updates {
                    data.price_store.add_price(&token_id, price);
                }
            }
            Err(e) => {
                warn!("Failed to update prices: {e}");
            }
        }

        time::sleep(poll_interval).await;
    }
}
