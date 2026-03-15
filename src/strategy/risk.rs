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
    mean_reversion_budget_pct: Decimal,
    tail_risk_budget_pct: Decimal,
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
            arb_budget_pct: config.arb_budget_pct,
            momentum_budget_pct: config.momentum_budget_pct,
            mean_reversion_budget_pct: config.mean_reversion_budget_pct,
            tail_risk_budget_pct: config.tail_risk_budget_pct,
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

        // Check cooldown (keyed on token_id to track per-outcome, not per-market)
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

        // Calculate available budget for this strategy
        let strategy_budget = match signal.source {
            SignalSource::Arbitrage => current_balance * self.arb_budget_pct,
            SignalSource::Momentum => current_balance * self.momentum_budget_pct,
            SignalSource::MeanReversion => current_balance * self.mean_reversion_budget_pct,
            SignalSource::TailRisk => current_balance * self.tail_risk_budget_pct,
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
            SignalSource::MeanReversion => {
                // Mean reversion: slightly larger bets (higher confidence in reversion)
                strategy_budget * dec!(0.08) // 8% of MR budget per trade
            }
            SignalSource::TailRisk => {
                // Kelly criterion: f = (payout * estimated_prob - 1) / (payout - 1)
                // estimated_prob = market_price * edge_multiplier (we assume market underprices tails)
                // Use half-Kelly to reduce variance
                if let crate::strategy::signals::SignalMetadata::TailRisk { payout_multiplier, outcome_price } = &signal.metadata {
                    let price_f64 = outcome_price.to_string().parse::<f64>().unwrap_or(0.05);
                    let estimated_prob = price_f64 * self.tail_risk_kelly_edge_multiplier;
                    let kelly = (payout_multiplier * estimated_prob - 1.0) / (payout_multiplier - 1.0);
                    let half_kelly = kelly / 2.0;
                    if half_kelly <= 0.0 {
                        return None; // No edge
                    }
                    let balance_f64 = current_balance.to_string().parse::<f64>().unwrap_or(0.0);
                    let kelly_bet = balance_f64 * half_kelly;
                    // Floor at tail_risk_bet_usd minimum
                    let bet = kelly_bet.max(self.tail_risk_bet_usd.to_string().parse::<f64>().unwrap_or(5.0));
                    Decimal::from_str_exact(&format!("{:.2}", bet)).unwrap_or(self.tail_risk_bet_usd)
                } else {
                    self.tail_risk_bet_usd
                }
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

    /// Sync budget percentages from a (possibly updated) config
    pub fn update_budgets(&mut self, config: &Config) {
        self.arb_budget_pct = config.arb_budget_pct;
        self.momentum_budget_pct = config.momentum_budget_pct;
        self.mean_reversion_budget_pct = config.mean_reversion_budget_pct;
        self.tail_risk_budget_pct = config.tail_risk_budget_pct;
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
