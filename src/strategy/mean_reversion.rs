use rust_decimal_macros::dec;
use tracing::debug;

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::{Signal, SignalDirection};

/// Scan all tracked markets for mean reversion signals.
///
/// The opposite of momentum: when price spikes, bet it reverts.
///
/// Strategy:
/// - If price rapidly ROSE → buy NO (expect reversion down)
/// - If price rapidly FELL → buy YES (expect reversion up)
pub async fn scan_mean_reversion(config: &Config, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let threshold = config
        .mean_reversion_threshold
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.20);
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        for outcome in &market.outcomes {
            // Skip outcomes with prices outside tradeable range
            if outcome.price < dec!(0.05) || outcome.price > dec!(0.95) {
                continue;
            }

            let derivative = match data.price_store.compute_derivative(&outcome.token_id) {
                Some(d) => d,
                None => continue,
            };

            let pct_change = derivative.pct_change;
            let abs_change = pct_change.abs();

            if abs_change >= threshold {
                // REVERSED direction compared to momentum:
                // Price spiked UP → expect reversion DOWN → BuyNo
                // Price spiked DOWN → expect reversion UP → BuyYes
                let (direction, edge_estimate) = if pct_change > 0.0 {
                    // Price rose fast → fade it, buy NO
                    (SignalDirection::BuyNo, abs_change * 40.0)
                } else {
                    // Price fell fast → fade it, buy YES
                    (SignalDirection::BuyYes, abs_change * 40.0)
                };

                debug!(
                    "MEAN REVERSION SIGNAL: {} | {} | change={:.1}% → fade with {:?}",
                    market.question,
                    outcome.name,
                    pct_change * 100.0,
                    direction,
                );

                let signal = Signal::new_mean_reversion(
                    market.condition_id.clone(),
                    market.question.clone(),
                    outcome.token_id.clone(),
                    outcome.name.clone(),
                    outcome.price,
                    direction,
                    edge_estimate,
                    pct_change,
                    derivative.derivative_per_sec,
                    derivative.num_points,
                );

                signals.push(signal);
            }
        }
    }

    signals
}
