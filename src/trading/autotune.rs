use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use tracing::debug;

use crate::config::Config;
use crate::trading::portfolio::Portfolio;
use crate::trading::testing::{linspace, TestConfig, TestPortfolio, TestStrategy};

/// Number of threshold points in the autotune sweep
const N_POINTS: usize = 10;

/// User-ID offset for autotune portfolios (distinct from test portfolios which use -1..-N).
/// These are never saved to DynamoDB so the value only needs to be unique in-process.
const AUTOTUNE_USER_ID_BASE: i64 = -1_000_000;

/// Build one paper portfolio per threshold point spanning [min, max].
/// Uses the configured `mr_bet_usd` so size-normalised ROI comparison is fair.
pub fn generate_autotune_portfolios(config: &Config) -> Vec<TestPortfolio> {
    let thresholds = linspace(
        config.mr_autotune_threshold_min,
        config.mr_autotune_threshold_max,
        N_POINTS,
    );
    thresholds
        .into_iter()
        .enumerate()
        .map(|(i, threshold)| {
            let thr_f64 = threshold.to_string().parse::<f64>().unwrap_or(0.20);
            TestPortfolio {
                portfolio: Portfolio::new(AUTOTUNE_USER_ID_BASE - i as i64, config.paper_balance, "paper"),
                config: TestConfig {
                    name: format!("autotune_mr_{:.3}", threshold),
                    bet_usd: config.mr_bet_usd,
                    strategy: TestStrategy::Mr,
                    threshold: thr_f64,
                },
                cooldowns: HashMap::new(),
            }
        })
        .collect()
}

/// Evaluate autotune portfolios over the last `window_hours` of closed trades.
///
/// Metric: ROI = total_realized_pnl / total_cost_basis for recent trades.
/// This normalises by amount invested so thresholds with different activity
/// levels can be compared fairly.
///
/// Returns `(best_threshold, roi_pct)` if at least one portfolio has recent
/// trades, otherwise `None`.
pub fn find_best_mr_threshold(
    portfolios: &[TestPortfolio],
    window_hours: i64,
) -> Option<(Decimal, Decimal)> {
    let cutoff = Utc::now() - Duration::hours(window_hours);

    let mut best_threshold: Option<Decimal> = None;
    let mut best_roi = Decimal::MIN;

    for tp in portfolios {
        let recent: Vec<_> = tp
            .portfolio
            .trade_history
            .iter()
            .filter(|t| t.closed_at >= cutoff)
            .collect();

        if recent.is_empty() {
            continue;
        }

        let total_pnl: Decimal = recent.iter().map(|t| t.realized_pnl).sum();
        // cost_basis ≈ quantity × entry_price (CompletedTrade has no stored cost_basis)
        let total_cost: Decimal = recent.iter().map(|t| t.quantity * t.entry_price).sum();

        if total_cost.is_zero() {
            continue;
        }

        let roi = total_pnl / total_cost;
        let wins = recent.iter().filter(|t| t.realized_pnl > Decimal::ZERO).count();

        debug!(
            "Autotune eval: thr={:.3} trades={} wins={} pnl=${:.2} roi={:.1}%",
            tp.config.threshold,
            recent.len(),
            wins,
            total_pnl,
            roi * dec!(100),
        );

        if roi > best_roi {
            best_roi = roi;
            best_threshold = format!("{:.4}", tp.config.threshold).parse().ok();
        }
    }

    best_threshold.map(|t| (t, best_roi * dec!(100)))
}

/// Refill all autotune portfolio balances back to `initial_balance` so each
/// evaluation window starts with equal capital. Trade history is kept so the
/// window filter still works correctly.
pub fn refill_balances(portfolios: &mut [TestPortfolio]) {
    for tp in portfolios.iter_mut() {
        tp.portfolio.balance = tp.portfolio.initial_balance;
    }
}
