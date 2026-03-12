use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{debug, info};

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::{ArbLeg, Signal};

/// Scan all tracked markets for arbitrage opportunities.
///
/// An arbitrage exists when the sum of the cheapest prices across all outcomes
/// is less than 1.0 (meaning you can buy all outcomes for less than the guaranteed payout).
pub async fn scan_arbitrage(config: &Config, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        if market.outcomes.len() < 2 {
            continue;
        }

        // Sum of all outcome prices
        let price_sum: Decimal = market.outcomes.iter().map(|o| o.price).sum();

        // Check if arbitrage exists: sum < threshold
        if price_sum < config.arb_threshold && price_sum > Decimal::ZERO {
            let edge = (dec!(1) - price_sum) / price_sum;
            let edge_f64 = edge.to_string().parse::<f64>().unwrap_or(0.0);

            // Build arbitrage legs — buy every outcome
            let arb_legs: Vec<ArbLeg> = market
                .outcomes
                .iter()
                .map(|o| ArbLeg {
                    token_id: o.token_id.clone(),
                    outcome_name: o.name.clone(),
                    price: o.price,
                })
                .collect();

            // Use the first outcome as the "primary" token for the signal
            let primary = &market.outcomes[0];

            let signal = Signal::new_arb(
                market.condition_id.clone(),
                market.question.clone(),
                primary.token_id.clone(),
                primary.name.clone(),
                primary.price,
                edge_f64 * 100.0, // as percentage
                price_sum,
                arb_legs,
            );

            info!(
                "ARB SIGNAL: {} | sum={:.4} | edge={:.2}%",
                market.question,
                price_sum,
                edge_f64 * 100.0
            );

            signals.push(signal);
        } else {
            debug!(
                "No arb: {} | sum={:.4}",
                market.question, price_sum
            );
        }
    }

    signals
}
