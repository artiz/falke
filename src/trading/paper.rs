use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::debug;
use uuid::Uuid;

use crate::strategy::signals::Signal;

use super::portfolio::{Portfolio, Position};

/// Paper trading engine — simulates order execution against real market data.
pub struct PaperTradingEngine {
    /// Simulated slippage as a fraction (0.01 = 1%)
    slippage: Decimal,
}

impl PaperTradingEngine {
    pub fn new() -> Self {
        Self {
            slippage: dec!(0.01),
        }
    }

    /// Execute a paper trade based on a signal
    pub fn execute_signal(
        &self,
        signal: &Signal,
        amount_usd: Decimal,
        portfolio: &mut Portfolio,
        use_take_profit: bool,
    ) -> Result<String, String> {
        let fill_price = signal.current_price * (dec!(1) + self.slippage);

        if fill_price <= Decimal::ZERO || fill_price >= dec!(1) {
            return Err(format!("Invalid fill price: {fill_price}"));
        }

        let quantity = amount_usd / fill_price;

        let position = Position {
            id: Uuid::new_v4().to_string(),
            condition_id: signal.condition_id.clone(),
            token_id: signal.token_id.clone(),
            question: signal.question.clone(),
            outcome_name: signal.outcome_name.clone(),
            side: "YES".to_string(),
            entry_price: fill_price,
            quantity,
            cost_basis: amount_usd,
            current_price: signal.current_price,
            source: signal.source.clone(),
            opened_at: Utc::now(),
            use_take_profit,
            imported: false,
            market_url: signal.market_url.clone(),
        };

        let position_id = position.id.clone();

        debug!(
            "PAPER TRADE: YES {} @ ${:.4} | qty={:.2} | cost=${:.2}",
            signal.outcome_name, fill_price, quantity, amount_usd,
        );

        portfolio.open_position(position)?;
        Ok(position_id)
    }
}
