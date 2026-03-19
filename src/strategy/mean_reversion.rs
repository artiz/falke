use rust_decimal_macros::dec;
use tracing::debug;

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::Signal;

/// Scan all tracked markets for mean reversion signals.
///
/// When a binary outcome's price spikes, bet the opposite (fade the move).
/// When a price crashes, bet it recovers.
///
/// - Price rose fast → buy the OTHER outcome (fade the spike)
/// - Price fell fast → buy THIS outcome (expect recovery)
pub async fn scan_mean_reversion(config: &Config, market_data: &SharedMarketData) -> Vec<Signal> {
    let data = market_data.read().await;
    let threshold = config
        .mean_reversion_threshold
        .to_string()
        .parse::<f64>()
        .unwrap_or(0.20);
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        if market.liquidity < config.min_liquidity_usd {
            continue;
        }

        for outcome in &market.outcomes {
            // Skip illiquid/near-resolved prices
            if outcome.price < dec!(0.05) || outcome.price > dec!(0.95) {
                continue;
            }

            let derivative = match data.price_store.compute_derivative(&outcome.token_id) {
                Some(d) => d,
                None => continue,
            };

            let pct_change = derivative.pct_change;
            let abs_change = pct_change.abs();

            if abs_change < threshold {
                continue;
            }

            if pct_change > 0.0 {
                // Price rose fast → fade it by buying the complementary outcome
                let complement = market
                    .outcomes
                    .iter()
                    .find(|o| o.token_id != outcome.token_id);
                if let Some(comp) = complement {
                    if comp.price >= dec!(0.05) && comp.price <= dec!(0.95) {
                        debug!(
                            "MR SIGNAL (fade rise): {} | {} rose {:.1}% → buy {} @ {:.3}",
                            market.question,
                            outcome.name,
                            pct_change * 100.0,
                            comp.name,
                            comp.price,
                        );
                        signals.push(Signal::new_mean_reversion(
                            market.condition_id.clone(),
                            market.question.clone(),
                            comp.token_id.clone(),
                            comp.name.clone(),
                            comp.price,
                            market.liquidity,
                            market.url_path(),
                            pct_change,
                        ));
                    }
                }
            } else {
                // Price fell fast → buy this outcome expecting recovery
                debug!(
                    "MR SIGNAL (fade fall): {} | {} fell {:.1}% → buy @ {:.3}",
                    market.question,
                    outcome.name,
                    pct_change * 100.0,
                    outcome.price,
                );
                signals.push(Signal::new_mean_reversion(
                    market.condition_id.clone(),
                    market.question.clone(),
                    outcome.token_id.clone(),
                    outcome.name.clone(),
                    outcome.price,
                    market.liquidity,
                    market.url_path(),
                    pct_change,
                ));
            }
        }
    }

    signals
}

/// Scan with the minimum threshold (for testing sweep — each test portfolio filters further).
pub async fn scan_mr_for_testing(
    min_threshold: f64,
    market_data: &SharedMarketData,
    min_liquidity_usd: rust_decimal::Decimal,
) -> Vec<Signal> {
    let data = market_data.read().await;
    let mut signals = Vec::new();

    for market in &data.tracked_markets {
        if market.liquidity < min_liquidity_usd {
            continue;
        }
        for outcome in &market.outcomes {
            if outcome.price < dec!(0.05) || outcome.price > dec!(0.95) {
                continue;
            }
            let derivative = match data.price_store.compute_derivative(&outcome.token_id) {
                Some(d) => d,
                None => continue,
            };
            let pct_change = derivative.pct_change;
            if pct_change.abs() < min_threshold {
                continue;
            }
            if pct_change > 0.0 {
                let complement = market
                    .outcomes
                    .iter()
                    .find(|o| o.token_id != outcome.token_id);
                if let Some(comp) = complement {
                    if comp.price >= dec!(0.05) && comp.price <= dec!(0.95) {
                        signals.push(Signal::new_mean_reversion(
                            market.condition_id.clone(),
                            market.question.clone(),
                            comp.token_id.clone(),
                            comp.name.clone(),
                            comp.price,
                            market.liquidity,
                            market.url_path(),
                            pct_change,
                        ));
                    }
                }
            } else {
                signals.push(Signal::new_mean_reversion(
                    market.condition_id.clone(),
                    market.question.clone(),
                    outcome.token_id.clone(),
                    outcome.name.clone(),
                    outcome.price,
                    market.liquidity,
                    market.url_path(),
                    pct_change,
                ));
            }
        }
    }

    signals
}
