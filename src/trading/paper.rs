#![allow(dead_code)]
use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::info;
use uuid::Uuid;

use crate::strategy::signals::{Signal, SignalDirection, SignalMetadata, SignalSource};

use super::portfolio::{Portfolio, Position};

/// Paper trading engine — simulates order execution against real market data.
///
/// In paper mode:
/// - Orders "fill" immediately at the signal's current price
/// - Slippage is simulated (configurable, default 1%)
/// - Positions auto-close when the market resolves or hits a stop/target
pub struct PaperTradingEngine {
    /// Simulated slippage as a fraction (0.01 = 1%)
    slippage: Decimal,
}

impl PaperTradingEngine {
    pub fn new() -> Self {
        Self {
            slippage: dec!(0.01), // 1% simulated slippage
        }
    }

    pub fn with_slippage(mut self, slippage: Decimal) -> Self {
        self.slippage = slippage;
        self
    }

    /// Execute a paper trade based on a signal
    pub fn execute_signal(
        &self,
        signal: &Signal,
        amount_usd: Decimal,
        portfolio: &mut Portfolio,
    ) -> Result<String, String> {
        // Apply simulated slippage
        let slippage_multiplier = dec!(1) + self.slippage;
        let fill_price = signal.current_price * slippage_multiplier;

        if fill_price <= Decimal::ZERO || fill_price >= dec!(1) {
            return Err(format!("Invalid fill price: {fill_price}"));
        }

        // Calculate quantity: amount / price
        let quantity = amount_usd / fill_price;

        let side = match signal.direction {
            SignalDirection::BuyYes => "YES",
            SignalDirection::BuyNo => "NO",
        };

        let position = Position {
            id: Uuid::new_v4().to_string(),
            condition_id: signal.condition_id.clone(),
            token_id: signal.token_id.clone(),
            question: signal.question.clone(),
            outcome_name: signal.outcome_name.clone(),
            side: side.to_string(),
            entry_price: fill_price,
            quantity,
            cost_basis: amount_usd,
            current_price: signal.current_price,
            source: signal.source.clone(),
            opened_at: Utc::now(),
        };

        let position_id = position.id.clone();

        info!(
            "PAPER TRADE: {} {} {} @ ${:.4} | qty={:.2} | cost=${:.2} | signal={}",
            side,
            signal.outcome_name,
            signal.question,
            fill_price,
            quantity,
            amount_usd,
            signal.source_label(),
        );

        portfolio.open_position(position)?;

        Ok(position_id)
    }

    /// Execute all legs of an arbitrage trade (buy all outcomes)
    pub fn execute_arb_signal(
        &self,
        signal: &Signal,
        total_amount_usd: Decimal,
        portfolio: &mut Portfolio,
    ) -> Result<Vec<String>, String> {
        let arb_legs = match &signal.metadata {
            SignalMetadata::Arbitrage { arb_legs, .. } => arb_legs.clone(),
            _ => return Err("Signal is not an arbitrage signal".to_string()),
        };

        if arb_legs.is_empty() {
            return Err("No arb legs found".to_string());
        }

        let price_sum: Decimal = arb_legs.iter().map(|l| l.price).sum();
        let mut position_ids = Vec::new();

        for leg in &arb_legs {
            // Allocate proportionally based on price
            let leg_amount = if price_sum > Decimal::ZERO {
                total_amount_usd * leg.price / price_sum
            } else {
                total_amount_usd / Decimal::from(arb_legs.len() as u32)
            };

            let fill_price = leg.price * (dec!(1) + self.slippage);
            let quantity = leg_amount / fill_price;

            let position = Position {
                id: Uuid::new_v4().to_string(),
                condition_id: signal.condition_id.clone(),
                token_id: leg.token_id.clone(),
                question: signal.question.clone(),
                outcome_name: leg.outcome_name.clone(),
                side: "ARB".to_string(),
                entry_price: fill_price,
                quantity,
                cost_basis: leg_amount,
                current_price: leg.price,
                source: SignalSource::Arbitrage,
                opened_at: Utc::now(),
            };

            let pid = position.id.clone();
            portfolio.open_position(position)?;
            position_ids.push(pid);
        }

        info!(
            "PAPER ARB: {} | {} legs | total=${:.2}",
            signal.question,
            arb_legs.len(),
            total_amount_usd,
        );

        Ok(position_ids)
    }
}

impl Signal {
    pub fn source_label(&self) -> &str {
        match self.source {
            SignalSource::Arbitrage => "ARB",
            SignalSource::Momentum => "MOM",
        }
    }
}
