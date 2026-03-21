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
    mr_bet_usd: Decimal,

    /// token_id -> last trade timestamp (for cooldown)
    cooldowns: HashMap<String, Instant>,
}

impl RiskManager {
    pub fn new(config: &Config) -> Self {
        Self {
            max_bet: config.max_bet_usd,
            max_open_positions: config.max_open_positions,
            cooldown_sec: config.cooldown_sec,
            mr_bet_usd: config.mean_reversion_bet_usd,
            cooldowns: HashMap::new(),
        }
    }

    /// Evaluate a MR/ML signal: fixed bet, no Kelly criterion.
    pub fn evaluate_mr(
        &self,
        signal: &Signal,
        current_balance: Decimal,
        open_positions: usize,
    ) -> Option<Decimal> {
        if open_positions >= self.max_open_positions {
            return None;
        }
        if let Some(last_trade) = self.cooldowns.get(&signal.token_id) {
            if last_trade.elapsed().as_secs() < self.cooldown_sec {
                debug!(
                    "Risk(MR): token {} in cooldown ({:.0}s remaining)",
                    signal.token_id,
                    self.cooldown_sec as f64 - last_trade.elapsed().as_secs_f64()
                );
                return None;
            }
        }
        let bet = self.mr_bet_usd.min(self.max_bet).max(dec!(1));
        if bet > current_balance {
            debug!("Risk(MR): insufficient balance for ${bet} trade");
            return None;
        }
        Some(bet)
    }

    /// Record that a trade was placed for cooldown tracking
    pub fn record_trade(&mut self, token_id: &str) {
        self.cooldowns.insert(token_id.to_string(), Instant::now());
    }

    /// Seed a cooldown entry with a specific Instant (used on startup to restore state)
    pub fn seed_cooldown(&mut self, token_id: String, at: Instant) {
        self.cooldowns.insert(token_id, at);
    }

    /// Clean up expired cooldowns
    pub fn cleanup_cooldowns(&mut self) {
        self.cooldowns
            .retain(|_, ts| ts.elapsed().as_secs() < self.cooldown_sec * 2);
    }
}
