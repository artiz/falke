use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::trading::portfolio::Portfolio;

#[derive(Clone, Debug)]
pub struct TestConfig {
    pub name: String,
    pub bet_usd: Decimal,
    /// MR threshold — all test portfolios are MR-mode
    pub mr_threshold: f64,
}

pub struct TestPortfolio {
    pub portfolio: Portfolio,
    pub config: TestConfig,
    /// Per-portfolio token cooldowns to avoid re-entering the same outcome
    pub cooldowns: HashMap<String, Instant>,
}

pub type SharedTestSessions = Arc<RwLock<Vec<TestPortfolio>>>;

pub fn new_shared_test_sessions() -> SharedTestSessions {
    Arc::new(RwLock::new(Vec::new()))
}

/// Generate 100 MR test portfolios: 10×10 sweep of threshold × bet.
pub fn generate_test_portfolios(config: &Config) -> Vec<TestPortfolio> {
    const POINTS: usize = 10;

    let thresholds =
        linspace(config.test_mr_threshold_min, config.test_mr_threshold_max, POINTS);
    let bets = linspace(config.test_mr_bet_usd_min, config.test_mr_bet_usd_max, POINTS);

    let mut out = Vec::with_capacity(POINTS * POINTS);
    let mut idx: i64 = 1;
    for &threshold in &thresholds {
        for &bet in &bets {
            let thr_f64 = threshold.to_string().parse::<f64>().unwrap_or(0.20);
            out.push(TestPortfolio {
                portfolio: Portfolio::new(-idx, config.paper_balance),
                config: TestConfig {
                    name: format!("mr_{:.3}_{:.1}", threshold, bet),
                    bet_usd: bet,
                    mr_threshold: thr_f64,
                },
                cooldowns: HashMap::new(),
            });
            idx += 1;
        }
    }
    out
}

pub(crate) fn linspace(start: Decimal, end: Decimal, n: usize) -> Vec<Decimal> {
    if n <= 1 {
        return vec![start];
    }
    let step = (end - start) / Decimal::from((n - 1) as u32);
    (0..n)
        .map(|i| start + step * Decimal::from(i as u32))
        .collect()
}
