#![allow(dead_code)]
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::strategy::signals::SignalSource;

/// A single position in the portfolio
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub id: String,
    pub condition_id: String,
    pub token_id: String,
    pub question: String,
    pub outcome_name: String,
    pub side: String, // "YES" or "NO"
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub cost_basis: Decimal,
    pub current_price: Decimal,
    pub source: SignalSource,
    pub opened_at: DateTime<Utc>,
    /// For tail risk: whether this position uses TP exit (vs hold to resolution)
    #[serde(default)]
    pub use_take_profit: bool,
}

impl Position {
    /// Unrealized P&L based on current price
    pub fn unrealized_pnl(&self) -> Decimal {
        (self.current_price - self.entry_price) * self.quantity
    }

    /// Unrealized P&L as a percentage
    pub fn unrealized_pnl_pct(&self) -> Decimal {
        if self.cost_basis == Decimal::ZERO {
            return Decimal::ZERO;
        }
        self.unrealized_pnl() / self.cost_basis * dec!(100)
    }
}

/// A completed trade (closed position)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTrade {
    pub id: String,
    pub condition_id: String,
    pub token_id: String,
    pub question: String,
    pub outcome_name: String,
    pub side: String,
    pub entry_price: Decimal,
    pub exit_price: Decimal,
    pub quantity: Decimal,
    pub realized_pnl: Decimal,
    pub realized_pnl_pct: Decimal,
    pub source: SignalSource,
    pub opened_at: DateTime<Utc>,
    pub closed_at: DateTime<Utc>,
    pub close_reason: String,
}

/// Per-user portfolio tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub user_id: i64,
    pub balance: Decimal,
    pub initial_balance: Decimal,
    pub open_positions: HashMap<String, Position>,
    pub trade_history: Vec<CompletedTrade>,
}

impl Portfolio {
    pub fn new(user_id: i64, initial_balance: Decimal) -> Self {
        Self {
            user_id,
            balance: initial_balance,
            initial_balance,
            open_positions: HashMap::new(),
            trade_history: Vec::new(),
        }
    }

    /// Open a new position
    pub fn open_position(&mut self, position: Position) -> Result<(), String> {
        if position.cost_basis > self.balance {
            return Err(format!(
                "Insufficient balance: need {}, have {}",
                position.cost_basis, self.balance
            ));
        }

        self.balance -= position.cost_basis;
        self.open_positions.insert(position.id.clone(), position);
        Ok(())
    }

    /// Close a position at the given exit price
    pub fn close_position(
        &mut self,
        position_id: &str,
        exit_price: Decimal,
        reason: &str,
    ) -> Result<CompletedTrade, String> {
        let position = self
            .open_positions
            .remove(position_id)
            .ok_or_else(|| format!("Position {position_id} not found"))?;

        let payout = exit_price * position.quantity;
        let realized_pnl = payout - position.cost_basis;
        let realized_pnl_pct = if position.cost_basis > Decimal::ZERO {
            realized_pnl / position.cost_basis * dec!(100)
        } else {
            Decimal::ZERO
        };

        self.balance += payout;

        let trade = CompletedTrade {
            id: position.id,
            condition_id: position.condition_id,
            token_id: position.token_id,
            question: position.question,
            outcome_name: position.outcome_name,
            side: position.side,
            entry_price: position.entry_price,
            exit_price,
            quantity: position.quantity,
            realized_pnl,
            realized_pnl_pct,
            source: position.source,
            opened_at: position.opened_at,
            closed_at: Utc::now(),
            close_reason: reason.to_string(),
        };

        self.trade_history.push(trade.clone());
        Ok(trade)
    }

    /// Update current prices for all open positions
    pub fn update_prices(&mut self, prices: &HashMap<String, Decimal>) {
        for position in self.open_positions.values_mut() {
            if let Some(&price) = prices.get(&position.token_id) {
                position.current_price = price;
            }
        }
    }

    /// Total portfolio value (balance + unrealized positions)
    pub fn total_value(&self) -> Decimal {
        let positions_value: Decimal = self
            .open_positions
            .values()
            .map(|p| p.current_price * p.quantity)
            .sum();
        self.balance + positions_value
    }

    /// Total realized P&L
    pub fn total_realized_pnl(&self) -> Decimal {
        self.trade_history.iter().map(|t| t.realized_pnl).sum()
    }

    /// Total unrealized P&L
    pub fn total_unrealized_pnl(&self) -> Decimal {
        self.open_positions
            .values()
            .map(|p| p.unrealized_pnl())
            .sum()
    }

    /// Number of open positions
    pub fn num_open_positions(&self) -> usize {
        self.open_positions.len()
    }

    /// Summary for display in Telegram
    pub fn summary(&self) -> String {
        let total = self.total_value();
        let total_pnl = total - self.initial_balance;
        let total_pnl_pct = if self.initial_balance > Decimal::ZERO {
            total_pnl / self.initial_balance * dec!(100)
        } else {
            Decimal::ZERO
        };

        let arb_pnl: Decimal = self
            .trade_history
            .iter()
            .filter(|t| t.source == SignalSource::Arbitrage)
            .map(|t| t.realized_pnl)
            .sum();

        let mom_pnl: Decimal = self
            .trade_history
            .iter()
            .filter(|t| t.source == SignalSource::Momentum)
            .map(|t| t.realized_pnl)
            .sum();

        let mr_pnl: Decimal = self
            .trade_history
            .iter()
            .filter(|t| t.source == SignalSource::MeanReversion)
            .map(|t| t.realized_pnl)
            .sum();

        let tail_pnl: Decimal = self
            .trade_history
            .iter()
            .filter(|t| t.source == SignalSource::TailRisk)
            .map(|t| t.realized_pnl)
            .sum();

        format!(
            "Portfolio Summary\n\
             ─────────────────\n\
             Balance: ${:.2}\n\
             Open positions: {}\n\
             Total value: ${:.2}\n\
             \n\
             Total P&L: ${:.2} ({:.1}%)\n\
             Arb P&L: ${:.2}\n\
             Momentum P&L: ${:.2}\n\
             Mean Rev P&L: ${:.2}\n\
             Tail Risk P&L: ${:.2}\n\
             Unrealized: ${:.2}\n\
             \n\
             Trades: {}",
            self.balance,
            self.num_open_positions(),
            total,
            total_pnl,
            total_pnl_pct,
            arb_pnl,
            mom_pnl,
            mr_pnl,
            tail_pnl,
            self.total_unrealized_pnl(),
            self.trade_history.len(),
        )
    }
}
