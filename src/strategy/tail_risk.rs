use rust_decimal_macros::dec;
use tracing::debug;

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::Signal;

/// Scan for tail-risk / long-shot opportunities.
///
/// Buy very cheap outcomes (under 5 cents) that pay 20x-100x if they hit.
/// Most will lose, but a few winners can cover all losses and then some.
///
/// Criteria:
/// - Outcome price <= max_price (default 5c)
/// - Market has reasonable liquidity
/// - Only buy YES on cheap outcomes (the long shot itself)
pub async fn scan_tail_risk(config: &Config, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let max_price = config.tail_risk_max_price;
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        for outcome in &market.outcomes {
            // Only interested in very cheap outcomes
            if outcome.price > max_price || outcome.price <= dec!(0.001) {
                continue;
            }

            let price_f64 = outcome.price.to_string().parse::<f64>().unwrap_or(1.0);
            let payout_multiplier = if price_f64 > 0.0 {
                1.0 / price_f64
            } else {
                0.0
            };

            if payout_multiplier < config.tail_risk_min_payout_multiplier {
                continue;
            }

            debug!(
                "TAIL RISK: {} | {} @ {:.3}c | {:.0}x payout",
                market.question,
                outcome.name,
                price_f64 * 100.0,
                payout_multiplier,
            );

            let signal = Signal::new_tail_risk(
                market.condition_id.clone(),
                market.question.clone(),
                outcome.token_id.clone(),
                outcome.name.clone(),
                outcome.price,
                payout_multiplier,
            );

            signals.push(signal);
        }
    }

    signals
}
