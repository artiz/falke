use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::debug;

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::Signal;

/// Scan for tail-risk / long-shot opportunities.
///
/// Buy very cheap outcomes that pay 25x-100x if they hit.
/// Most will lose, but a few winners cover all losses and then some.
pub async fn scan_tail_risk(config: &Config, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let max_price = config.tail_risk_max_price;
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        for outcome in &market.outcomes {
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

            signals.push(Signal::new_tail_risk(
                market.condition_id.clone(),
                market.question.clone(),
                outcome.token_id.clone(),
                outcome.name.clone(),
                outcome.price,
                market.liquidity,
                payout_multiplier,
                market.url_path(),
            ));
        }
    }

    // Sort: lowest price first (highest payout), then highest liquidity
    signals.sort_by(|a, b| {
        a.current_price
            .cmp(&b.current_price)
            .then(b.liquidity.cmp(&a.liquidity))
    });

    signals
}

/// Broader scan for the testing engine — returns every outcome at or below `max_price`
/// with no `min_payout_multiplier` filter. Each test portfolio applies its own price
/// filter at entry time.
pub async fn scan_for_testing(max_price: Decimal, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        for outcome in &market.outcomes {
            if outcome.price > max_price || outcome.price <= dec!(0.001) {
                continue;
            }
            let price_f64 = outcome.price.to_string().parse::<f64>().unwrap_or(1.0);
            let payout_multiplier = if price_f64 > 0.0 {
                1.0 / price_f64
            } else {
                0.0
            };
            signals.push(Signal::new_tail_risk(
                market.condition_id.clone(),
                market.question.clone(),
                outcome.token_id.clone(),
                outcome.name.clone(),
                outcome.price,
                market.liquidity,
                payout_multiplier,
                market.url_path(),
            ));
        }
    }
    signals
}
