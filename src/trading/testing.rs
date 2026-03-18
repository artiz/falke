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
    pub max_price: Decimal,
    pub bet_usd: Decimal,
    pub take_profit_pct: Decimal,
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

/// Generate 10×10×10 = 1000 test portfolios covering all combinations of the three ranges.
pub fn generate_test_portfolios(config: &Config) -> Vec<TestPortfolio> {
    const POINTS: usize = 10;
    let prices = linspace(config.test_max_price_min, config.test_max_price_max, POINTS);
    let bets = linspace(config.test_bet_usd_min, config.test_bet_usd_max, POINTS);
    let tps = if config.test_take_profit_pct_min == Decimal::ZERO
        && config.test_take_profit_pct_max == Decimal::ZERO
    {
        vec![Decimal::ZERO]
    } else {
        linspace(
            config.test_take_profit_pct_min,
            config.test_take_profit_pct_max,
            POINTS,
        )
    };

    let mut out = Vec::with_capacity(POINTS * POINTS * POINTS);
    let mut idx: i64 = 1;
    for &price in &prices {
        for &bet in &bets {
            for &tp in &tps {
                out.push(TestPortfolio {
                    portfolio: Portfolio::new(-idx, config.paper_balance),
                    config: TestConfig {
                        name: format!("st_{:.3}_{:.1}_{:.0}", price, bet, tp),
                        max_price: price,
                        bet_usd: bet,
                        take_profit_pct: tp,
                    },
                    cooldowns: HashMap::new(),
                });
                idx += 1;
            }
        }
    }
    out
}

fn linspace(start: Decimal, end: Decimal, n: usize) -> Vec<Decimal> {
    if n <= 1 {
        return vec![start];
    }
    let step = (end - start) / Decimal::from((n - 1) as u32);
    (0..n)
        .map(|i| start + step * Decimal::from(i as u32))
        .collect()
}
