use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::trading::portfolio::Portfolio;

#[derive(Clone, Debug)]
pub enum TestStrategy {
    /// Plain mean-reversion; threshold is the pct_change cutoff (e.g. 0.20)
    Mr,
    /// ML-filtered MR; threshold is the win_prob cutoff (e.g. 0.60)
    Ml,
}

#[derive(Clone, Debug)]
pub struct TestConfig {
    pub name: String,
    pub bet_usd: Decimal,
    pub strategy: TestStrategy,
    /// Threshold: pct_change for Mr, win_prob for Ml
    pub threshold: f64,
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

/// Generate test portfolios:
///   - 100 MR portfolios: 10×10 sweep of pct_change threshold × bet
///   - 100 ML portfolios: 10×10 sweep of win_prob threshold × bet
pub fn generate_test_portfolios(config: &Config) -> Vec<TestPortfolio> {
    const POINTS: usize = 10;

    let mr_thresholds =
        linspace(config.test_mr_threshold_min, config.test_mr_threshold_max, POINTS);
    let ml_thresholds = linspace_f64(config.test_ml_threshold_min, config.test_ml_threshold_max, POINTS);
    let bets = linspace(config.test_mr_bet_usd_min, config.test_mr_bet_usd_max, POINTS);

    let mut out = Vec::with_capacity(POINTS * POINTS * 2);
    let mut idx: i64 = 1;

    // MR portfolios (negative IDs -1 .. -100)
    for &threshold in &mr_thresholds {
        for &bet in &bets {
            let thr_f64 = threshold.to_string().parse::<f64>().unwrap_or(0.20);
            out.push(TestPortfolio {
                portfolio: Portfolio::new(-idx, config.paper_balance),
                config: TestConfig {
                    name: format!("mr_{:.3}_{:.1}", threshold, bet),
                    bet_usd: bet,
                    strategy: TestStrategy::Mr,
                    threshold: thr_f64,
                },
                cooldowns: HashMap::new(),
            });
            idx += 1;
        }
    }

    // ML portfolios (negative IDs -101 .. -200)
    for &thr in &ml_thresholds {
        for &bet in &bets {
            out.push(TestPortfolio {
                portfolio: Portfolio::new(-idx, config.paper_balance),
                config: TestConfig {
                    name: format!("ml_{:.2}_{:.1}", thr, bet),
                    bet_usd: bet,
                    strategy: TestStrategy::Ml,
                    threshold: thr,
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

pub(crate) fn linspace_f64(start: f64, end: f64, n: usize) -> Vec<f64> {
    if n <= 1 {
        return vec![start];
    }
    let step = (end - start) / (n - 1) as f64;
    (0..n).map(|i| start + step * i as f64).collect()
}
