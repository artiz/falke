use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
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
    /// Persistent cache: condition_id → url_path. Accumulates entries, never cleared.
    pub slug_cache: HashMap<String, String>,
    /// Set of condition_ids belonging to ignored topics (e.g. politics).
    pub ignored_condition_ids: HashSet<String>,
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
        slug_cache: HashMap::new(),
        ignored_condition_ids: HashSet::new(),
    }))
}

/// The market data collector runs as a background task.
/// It periodically fetches markets and their prices, updating the shared state.
pub async fn run_collector(config: Config, market_data: SharedMarketData) {
    let gamma = GammaClient::new(&config.gamma_api_url);
    let poll_interval = Duration::from_secs(config.trade_poll_interval_sec);

    // Fetch market list less frequently (every 60 x trade_poll_interval_sec)
    let market_refresh_interval = Duration::from_secs(config.trade_poll_interval_sec * 60);
    let mut last_market_refresh = std::time::Instant::now() - market_refresh_interval;

    info!(
        "Market data collector started. Poll interval: {}s, Expiry window: {}h",
        config.trade_poll_interval_sec, config.market_expiry_window_hours
    );

    loop {
        // Refresh market list periodically
        let now = std::time::Instant::now();
        if now.duration_since(last_market_refresh) >= market_refresh_interval {
            match gamma
                .fetch_expiring_markets(config.market_expiry_window_hours)
                .await
            {
                Ok(gamma_markets) => {
                    // Refresh ignored condition IDs before building tracked list
                    let new_ignored = if !config.ignored_topics.is_empty() {
                        gamma
                            .fetch_ignored_condition_ids(&config.ignored_topics)
                            .await
                    } else {
                        HashSet::new()
                    };

                    let tracked: Vec<TrackedMarket> = gamma_markets
                        .iter()
                        .filter_map(|m| GammaClient::to_tracked_market(m))
                        .filter(|m| m.liquidity >= config.min_liquidity_usd)
                        .filter(|m| !new_ignored.contains(&m.condition_id))
                        .collect();

                    info!("Refreshed market list: {} markets tracked", tracked.len());

                    let mut data = market_data.write().await;

                    // Update ignored set
                    if !config.ignored_topics.is_empty() {
                        data.ignored_condition_ids = new_ignored;
                    }

                    // Update persistent slug cache with newly tracked markets
                    for m in &tracked {
                        if let Some(url) = m.url_path() {
                            data.slug_cache.insert(m.condition_id.clone(), url);
                        }
                    }

                    // Also populate slug cache from a wider window (7 days) so that
                    // open positions in markets outside the signal window still get links.
                    // We do this opportunistically — errors here are non-fatal.
                    if let Ok(wide) = gamma.fetch_expiring_markets(7 * 24).await {
                        for m in &wide {
                            if let Some(slug) = m.slug.as_ref() {
                                let group = m.events.first().and_then(|e| e.slug.as_ref());
                                let url = match group {
                                    Some(g) if g != slug => format!("{}/{}", g, slug),
                                    _ => slug.clone(),
                                };
                                data.slug_cache.insert(m.condition_id.clone(), url);
                            }
                        }
                    }

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
            .fetch_expiring_markets(config.market_expiry_window_hours)
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
