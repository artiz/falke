use rust_decimal_macros::dec;
use tracing::{debug, info};

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::{Signal, SignalDirection};

/// Scan all tracked markets for momentum signals.
///
/// A momentum signal fires when the price derivative over the last 5 minutes
/// exceeds the configured threshold (e.g., 30% change).
///
/// Strategy:
/// - If price is rapidly RISING → buy YES (ride the momentum up)
/// - If price is rapidly FALLING → buy NO (ride the momentum down)
pub async fn scan_momentum(config: &Config, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let threshold = config
        .momentum_derivative_threshold
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.30);
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
                let (direction, edge_estimate) = if pct_change > 0.0 {
                    // Price rising fast → buy YES
                    // Edge estimate: assume momentum continues for ~50% of observed change
                    (SignalDirection::BuyYes, abs_change * 50.0)
                } else {
                    // Price falling fast → buy NO
                    (SignalDirection::BuyNo, abs_change * 50.0)
                };

                let signal = Signal::new_momentum(
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

                info!(
                    "MOMENTUM SIGNAL: {} | {} | change={:.1}% in {:.0}s | {} points",
                    market.question,
                    outcome.name,
                    pct_change * 100.0,
                    derivative.window_sec,
                    derivative.num_points,
                );

                signals.push(signal);
            } else {
                debug!(
                    "No momentum: {} {} | change={:.2}%",
                    market.question,
                    outcome.name,
                    pct_change * 100.0,
                );
            }
        }
    }

    signals
}
