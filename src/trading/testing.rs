use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::config::Config;
use crate::trading::portfolio::Portfolio;

#[derive(Clone, Debug)]
pub struct TestConfig {
    pub name: String,
    pub max_price: Decimal,
    pub bet_usd: Decimal,
    /// Some(threshold) = MR-mode portfolio; None = tail-risk-mode portfolio
    pub mr_threshold: Option<f64>,
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

/// Generate test portfolios.
///
/// - If `mean_reversion_budget_pct == 1.0`: sweep MR threshold × bet (MR-only mode).
/// - Otherwise: sweep tail-risk max_price × bet (TR mode).
///
/// Either way produces 10×10 = 100 portfolios.
pub fn generate_test_portfolios(config: &Config) -> Vec<TestPortfolio> {
    const POINTS: usize = 10;

    if config.mean_reversion_budget_pct >= dec!(1.0) {
        // MR-only sweep: threshold × bet
        let thresholds =
            linspace(config.test_mr_threshold_min, config.test_mr_threshold_max, POINTS);
        let bets = linspace(config.test_bet_usd_min, config.test_bet_usd_max, POINTS);

        let mut out = Vec::with_capacity(POINTS * POINTS);
        let mut idx: i64 = 1;
        for &threshold in &thresholds {
            for &bet in &bets {
                let thr_f64 = threshold.to_string().parse::<f64>().unwrap_or(0.20);
                out.push(TestPortfolio {
                    portfolio: Portfolio::new(-idx, config.paper_balance),
                    config: TestConfig {
                        name: format!("mr_{:.3}_{:.1}", threshold, bet),
                        max_price: Decimal::ZERO, // unused in MR mode
                        bet_usd: bet,
                        mr_threshold: Some(thr_f64),
                    },
                    cooldowns: HashMap::new(),
                });
                idx += 1;
            }
        }
        out
    } else {
        // Tail-risk sweep: max_price × bet
        let prices = linspace(config.test_max_price_min, config.test_max_price_max, POINTS);
        let bets = linspace(config.test_bet_usd_min, config.test_bet_usd_max, POINTS);

        let mut out = Vec::with_capacity(POINTS * POINTS);
        let mut idx: i64 = 1;
        for &price in &prices {
            for &bet in &bets {
                out.push(TestPortfolio {
                    portfolio: Portfolio::new(-idx, config.paper_balance),
                    config: TestConfig {
                        name: format!("st_{:.3}_{:.1}", price, bet),
                        max_price: price,
                        bet_usd: bet,
                        mr_threshold: None,
                    },
                    cooldowns: HashMap::new(),
                });
                idx += 1;
            }
        }
        out
    }
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
