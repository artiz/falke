use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::time::Instant;
use tracing::debug;

use crate::config::Config;

use super::signals::Signal;

/// Risk manager that controls position sizing, exposure limits, and cooldowns.
pub struct RiskManager {
    max_bet: Decimal,
    max_open_positions: usize,
    cooldown_sec: u64,
    tail_risk_bet_usd: Decimal,
    tail_risk_kelly_edge_multiplier: f64,

    /// token_id -> last trade timestamp (for cooldown)
    cooldowns: HashMap<String, Instant>,
}

impl RiskManager {
    pub fn new(config: &Config) -> Self {
        Self {
            max_bet: config.max_bet_usd,
            max_open_positions: config.max_open_positions,
            cooldown_sec: config.cooldown_sec,
            tail_risk_bet_usd: config.tail_risk_bet_usd,
            tail_risk_kelly_edge_multiplier: config.tail_risk_kelly_edge_multiplier,
            cooldowns: HashMap::new(),
        }
    }

    /// Check if a signal passes risk checks and return the position size
    pub fn evaluate(
        &self,
        signal: &Signal,
        current_balance: Decimal,
        open_positions: usize,
    ) -> Option<Decimal> {
        // Check position limit
        if open_positions >= self.max_open_positions {
            return None;
        }

        // Check cooldown (keyed on token_id to track per-outcome)
        if let Some(last_trade) = self.cooldowns.get(&signal.token_id) {
            if last_trade.elapsed().as_secs() < self.cooldown_sec {
                debug!(
                    "Risk: token {} in cooldown ({:.0}s remaining)",
                    signal.token_id,
                    self.cooldown_sec as f64 - last_trade.elapsed().as_secs_f64()
                );
                return None;
            }
        }

        // Kelly criterion: f = (payout * estimated_prob - 1) / (payout - 1)
        // estimated_prob = market_price * edge_multiplier
        // Use half-Kelly to reduce variance
        let price_f64 = signal.current_price.to_string().parse::<f64>().unwrap_or(0.05);
        let estimated_prob = price_f64 * self.tail_risk_kelly_edge_multiplier;
        let kelly = (signal.payout_multiplier * estimated_prob - 1.0) / (signal.payout_multiplier - 1.0);
        let half_kelly = kelly / 2.0;
        if half_kelly <= 0.0 {
            return None; // No edge
        }

        let balance_f64 = current_balance.to_string().parse::<f64>().unwrap_or(0.0);
        let kelly_bet = balance_f64 * half_kelly;
        let min_bet = self.tail_risk_bet_usd.to_string().parse::<f64>().unwrap_or(5.0);
        let bet = RustDecimal::from_str(&format!("{:.2}", kelly_bet.max(min_bet)))
            .unwrap_or(self.tail_risk_bet_usd);

        let position_size = bet.min(self.max_bet).max(dec!(1));

        if position_size > current_balance {
            debug!("Risk: insufficient balance for ${position_size} trade");
            return None;
        }

        Some(position_size)
    }

    /// Record that a trade was placed for cooldown tracking
    pub fn record_trade(&mut self, token_id: &str) {
        self.cooldowns.insert(token_id.to_string(), Instant::now());
    }

    /// Clean up expired cooldowns
    pub fn cleanup_cooldowns(&mut self) {
        self.cooldowns
            .retain(|_, ts| ts.elapsed().as_secs() < self.cooldown_sec * 2);
    }
}

use rust_decimal::prelude::FromStr;
use rust_decimal::Decimal as RustDecimal;
