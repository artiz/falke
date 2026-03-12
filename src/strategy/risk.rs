#![allow(dead_code)]
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::time::Instant;
use tracing::debug;

use crate::config::Config;

use super::signals::{Signal, SignalSource};

/// Risk manager that controls position sizing, exposure limits, and cooldowns.
pub struct RiskManager {
    max_bet: Decimal,
    max_open_positions: usize,
    cooldown_sec: u64,
    arb_budget_pct: Decimal,
    momentum_budget_pct: Decimal,

    /// market_condition_id -> last trade timestamp (for cooldown)
    cooldowns: HashMap<String, Instant>,
}

impl RiskManager {
    pub fn new(config: &Config) -> Self {
        Self {
            max_bet: config.max_bet_usd,
            max_open_positions: config.max_open_positions,
            cooldown_sec: config.cooldown_sec,
            arb_budget_pct: config.arb_budget_pct,
            momentum_budget_pct: config.momentum_budget_pct,
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
            debug!(
                "Risk: position limit reached ({}/{})",
                open_positions, self.max_open_positions
            );
            return None;
        }

        // Check cooldown
        if let Some(last_trade) = self.cooldowns.get(&signal.condition_id) {
            if last_trade.elapsed().as_secs() < self.cooldown_sec {
                debug!(
                    "Risk: market {} in cooldown ({:.0}s remaining)",
                    signal.condition_id,
                    self.cooldown_sec as f64 - last_trade.elapsed().as_secs_f64()
                );
                return None;
            }
        }

        // Calculate available budget for this strategy
        let strategy_budget = match signal.source {
            SignalSource::Arbitrage => current_balance * self.arb_budget_pct,
            SignalSource::Momentum => current_balance * self.momentum_budget_pct,
        };

        if strategy_budget <= Decimal::ZERO {
            return None;
        }

        // Position size: min of max_bet and a fraction of strategy budget
        // For arb, use larger positions; for momentum, use smaller
        let base_size = match signal.source {
            SignalSource::Arbitrage => {
                // Arb: size proportional to edge (more edge = bigger bet)
                let edge_factor =
                    Decimal::from_str_exact(&format!("{:.4}", signal.estimated_edge_pct / 100.0))
                        .unwrap_or(dec!(0.01));
                strategy_budget * edge_factor * dec!(10) // Scale up since edges are small
            }
            SignalSource::Momentum => {
                // Momentum: fixed small bets
                strategy_budget * dec!(0.05) // 5% of momentum budget per trade
            }
        };

        let position_size = base_size.min(self.max_bet).max(dec!(1)); // Min $1, max max_bet

        // Ensure we have enough balance
        if position_size > current_balance {
            debug!("Risk: insufficient balance for ${position_size} trade");
            return None;
        }

        Some(position_size)
    }

    /// Record that a trade was placed for cooldown tracking
    pub fn record_trade(&mut self, condition_id: &str) {
        self.cooldowns
            .insert(condition_id.to_string(), Instant::now());
    }

    /// Clean up expired cooldowns
    pub fn cleanup_cooldowns(&mut self) {
        self.cooldowns
            .retain(|_, ts| ts.elapsed().as_secs() < self.cooldown_sec * 2);
    }
}

use rust_decimal::prelude::FromStr;
use rust_decimal::Decimal as RustDecimal;

trait DecimalFromStr {
    fn from_str_exact(s: &str) -> Result<Decimal, rust_decimal::Error>;
}

impl DecimalFromStr for Decimal {
    fn from_str_exact(s: &str) -> Result<Decimal, rust_decimal::Error> {
        RustDecimal::from_str(s)
    }
}
