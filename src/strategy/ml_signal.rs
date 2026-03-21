use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;
use rust_decimal_macros::dec;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::market_data::collector::SharedMarketData;

use super::signals::Signal;

/// XGBoost ML filter loaded from an ONNX model file.
///
/// Scores MR signals using the model trained in research/analysis.ipynb.
/// Features (order matches training):
///   [abs_pct_change, entry_price, dist_from_50, potential_payout,
///    log_volume, volatility, direction_enc, topic_enc, market_type_enc]
///
/// topic_enc is always 0 (unknown) — the Gamma API does not return category data
/// in market responses, only as a query filter, so it cannot be stored per-market.
/// market_type_enc is always 0 (binary) — we only trade binary markets.
/// log_volume uses market liquidity as a proxy for trading volume.
pub struct MlFilter {
    session: Mutex<Session>,
    pub threshold: f64,
}

impl MlFilter {
    pub fn load(model_path: &str, threshold: f64) -> anyhow::Result<Self> {
        info!("Loading ML model from {model_path}");
        let session = Session::builder()?.commit_from_file(model_path)?;
        info!("ML model loaded — win-prob threshold: {threshold:.2}");
        Ok(Self {
            session: Mutex::new(session),
            threshold,
        })
    }

    fn predict_inner(&self, features: [f32; 9]) -> anyhow::Result<f64> {
        let tensor =
            Tensor::<f32>::from_array((vec![1i64, 9], features.to_vec()))?;
        let mut session = self.session.lock().expect("ML session mutex poisoned");
        let outputs = session.run(ort::inputs!["features" => tensor])?;
        // outputs[1] = probabilities, shape [1, 2]; data[1] = P(win)
        let (_, data) = outputs[1].try_extract_tensor::<f32>()?;
        Ok(data[1] as f64)
    }

    pub fn predict(&self, features: [f32; 9]) -> f64 {
        self.predict_inner(features).unwrap_or_else(|e| {
            warn!("ML inference error: {e}");
            0.0
        })
    }
}

fn compute_volatility(prices: &[f64]) -> f64 {
    if prices.len() < 2 {
        return 0.0;
    }
    let returns: Vec<f64> = prices
        .windows(2)
        .filter_map(|w| if w[0] > 0.0 { Some((w[1] - w[0]) / w[0]) } else { None })
        .collect();
    if returns.is_empty() {
        return 0.0;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let var = returns
        .iter()
        .map(|r| (r - mean).powi(2))
        .sum::<f64>()
        / returns.len() as f64;
    var.sqrt()
}

fn decimal_to_f64(d: rust_decimal::Decimal) -> f64 {
    d.to_string().parse().unwrap_or(0.0)
}

/// Scan for MR signals that pass the XGBoost ML filter.
///
/// Identical entry logic to `scan_mean_reversion` but each candidate signal is
/// scored by the ONNX model before being emitted. Only signals with
/// `win_prob >= filter.threshold` are returned.
///
/// MARKET_EXPIRY_WINDOW_HOURS is respected automatically because the engine's
/// market collector already filters `tracked_markets` by expiry window.
pub async fn scan_ml_filtered(
    config: &Config,
    market_data: &SharedMarketData,
    filter: &MlFilter,
) -> Vec<Signal> {
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
        let liquidity_f64 = decimal_to_f64(market.liquidity);
        let log_volume = (1.0_f64 + liquidity_f64).ln() as f32;
        // market_type_enc: always 0 (binary) — collector only returns 2-outcome markets
        let market_type_enc = 0.0_f32;
        // topic_enc: 0 (unknown) — topic not stored in TrackedMarket
        let topic_enc = 0.0_f32;

        for outcome in &market.outcomes {
            if outcome.price < dec!(0.05) || outcome.price > dec!(0.95) {
                continue;
            }

            let derivative = match data.price_store.compute_derivative(&outcome.token_id) {
                Some(d) => d,
                None => continue,
            };

            let pct_change = derivative.pct_change;
            let abs_pct = pct_change.abs();
            if abs_pct < threshold {
                continue;
            }

            let recent_prices: Vec<f64> = data
                .price_store
                .get_recent(&outcome.token_id)
                .iter()
                .map(|p| decimal_to_f64(p.price))
                .collect();
            let volatility = compute_volatility(&recent_prices) as f32;

            if pct_change > 0.0 {
                // Price rose → fade it by buying the complementary outcome
                let complement = market
                    .outcomes
                    .iter()
                    .find(|o| o.token_id != outcome.token_id);
                if let Some(comp) = complement {
                    if comp.price < dec!(0.05) || comp.price > dec!(0.95) {
                        continue;
                    }
                    let entry = decimal_to_f64(comp.price) as f32;
                    let features = [
                        abs_pct as f32,
                        entry,
                        (entry - 0.5).abs(),
                        if entry > 0.0 { 1.0 / entry } else { 0.0 },
                        log_volume,
                        volatility,
                        1.0_f32, // direction_enc: fade_rise = 1
                        topic_enc,
                        market_type_enc,
                    ];
                    let win_prob = filter.predict(features);
                    if win_prob < filter.threshold {
                        debug!(
                            "ML SKIP (fade_rise): {} rose {:.0}% → comp @ {:.3} prob={:.2}",
                            market.question,
                            pct_change * 100.0,
                            comp.price,
                            win_prob
                        );
                        continue;
                    }
                    debug!(
                        "ML SIGNAL (fade_rise): {} | {} rose {:.0}% → buy {} @ {:.3} prob={:.2}",
                        market.question,
                        outcome.name,
                        pct_change * 100.0,
                        comp.name,
                        comp.price,
                        win_prob
                    );
                    signals.push(Signal::new_ml_filtered(
                        market.condition_id.clone(),
                        market.question.clone(),
                        comp.token_id.clone(),
                        comp.name.clone(),
                        comp.price,
                        market.liquidity,
                        market.url_path(),
                        pct_change,
                        win_prob,
                    ));
                }
            } else {
                // Price fell → buy this outcome expecting recovery
                let entry = decimal_to_f64(outcome.price) as f32;
                let features = [
                    abs_pct as f32,
                    entry,
                    (entry - 0.5).abs(),
                    if entry > 0.0 { 1.0 / entry } else { 0.0 },
                    log_volume,
                    volatility,
                    0.0_f32, // direction_enc: fade_fall = 0
                    topic_enc,
                    market_type_enc,
                ];
                let win_prob = filter.predict(features);
                if win_prob < filter.threshold {
                    debug!(
                        "ML SKIP (fade_fall): {} fell {:.0}% @ {:.3} prob={:.2}",
                        market.question,
                        pct_change * 100.0,
                        outcome.price,
                        win_prob
                    );
                    continue;
                }
                debug!(
                    "ML SIGNAL (fade_fall): {} | {} fell {:.0}% @ {:.3} prob={:.2}",
                    market.question,
                    outcome.name,
                    pct_change * 100.0,
                    outcome.price,
                    win_prob
                );
                signals.push(Signal::new_ml_filtered(
                    market.condition_id.clone(),
                    market.question.clone(),
                    outcome.token_id.clone(),
                    outcome.name.clone(),
                    outcome.price,
                    market.liquidity,
                    market.url_path(),
                    pct_change,
                    win_prob,
                ));
            }
        }
    }

    signals
}

/// Scan ML-filtered signals for test portfolios using an explicit minimum threshold.
/// Returns all signals with win_prob >= min_threshold so each test portfolio can
/// apply its own per-portfolio threshold on top.
pub async fn scan_ml_for_testing(
    min_threshold: f64,
    config: &Config,
    market_data: &SharedMarketData,
    filter: &MlFilter,
) -> Vec<Signal> {
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
        let liquidity_f64 = decimal_to_f64(market.liquidity);
        let log_volume = (1.0_f64 + liquidity_f64).ln() as f32;
        let market_type_enc = 0.0_f32;
        let topic_enc = 0.0_f32;

        for outcome in &market.outcomes {
            if outcome.price < dec!(0.05) || outcome.price > dec!(0.95) {
                continue;
            }

            let derivative = match data.price_store.compute_derivative(&outcome.token_id) {
                Some(d) => d,
                None => continue,
            };

            let pct_change = derivative.pct_change;
            let abs_pct = pct_change.abs();
            if abs_pct < threshold {
                continue;
            }

            let recent_prices: Vec<f64> = data
                .price_store
                .get_recent(&outcome.token_id)
                .iter()
                .map(|p| decimal_to_f64(p.price))
                .collect();
            let volatility = compute_volatility(&recent_prices) as f32;

            if pct_change > 0.0 {
                let complement = market
                    .outcomes
                    .iter()
                    .find(|o| o.token_id != outcome.token_id);
                if let Some(comp) = complement {
                    if comp.price < dec!(0.05) || comp.price > dec!(0.95) {
                        continue;
                    }
                    let entry = decimal_to_f64(comp.price) as f32;
                    let features = [
                        abs_pct as f32,
                        entry,
                        (entry - 0.5).abs(),
                        if entry > 0.0 { 1.0 / entry } else { 0.0 },
                        log_volume,
                        volatility,
                        1.0_f32,
                        topic_enc,
                        market_type_enc,
                    ];
                    let win_prob = filter.predict(features);
                    if win_prob < min_threshold {
                        continue;
                    }
                    signals.push(Signal::new_ml_filtered(
                        market.condition_id.clone(),
                        market.question.clone(),
                        comp.token_id.clone(),
                        comp.name.clone(),
                        comp.price,
                        market.liquidity,
                        market.url_path(),
                        pct_change,
                        win_prob,
                    ));
                }
            } else {
                let entry = decimal_to_f64(outcome.price) as f32;
                let features = [
                    abs_pct as f32,
                    entry,
                    (entry - 0.5).abs(),
                    if entry > 0.0 { 1.0 / entry } else { 0.0 },
                    log_volume,
                    volatility,
                    0.0_f32,
                    topic_enc,
                    market_type_enc,
                ];
                let win_prob = filter.predict(features);
                if win_prob < min_threshold {
                    continue;
                }
                signals.push(Signal::new_ml_filtered(
                    market.condition_id.clone(),
                    market.question.clone(),
                    outcome.token_id.clone(),
                    outcome.name.clone(),
                    outcome.price,
                    market.liquidity,
                    market.url_path(),
                    pct_change,
                    win_prob,
                ));
            }
        }
    }

    signals
}
