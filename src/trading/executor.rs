use rust_decimal::Decimal;
use tracing::{debug, info};

use crate::error::{FalkeError, Result};
use crate::polymarket::clob_api::{ClobClient, OrderSide};
use crate::strategy::signals::Signal;

/// Live trading executor — places real orders via the Polymarket CLOB API.
pub struct LiveExecutor {
    clob: ClobClient,
}

impl LiveExecutor {
    pub fn new(clob: ClobClient) -> Self {
        Self { clob }
    }

    /// Place a limit order for a given signal.
    /// `amount_usd` is the USD value to spend; the SDK expects token quantity = amount_usd / price.
    pub async fn execute_signal(&self, signal: &Signal, amount_usd: Decimal) -> Result<String> {
        let price = signal.current_price.round_dp(3);
        if price == Decimal::ZERO {
            return Err(FalkeError::OrderRejected("price is zero".into()));
        }

        // SDK limit_order().size() = number of outcome tokens, not USD
        let token_qty = (amount_usd / price).round_dp(2);

        debug!(
            "LIVE ORDER: Buy {} @ {} | qty={} (${:.2})",
            signal.outcome_name, price, token_qty, amount_usd
        );

        self.clob
            .place_order(&signal.token_id, OrderSide::Buy, price, token_qty)
            .await
    }

    /// Place a sell limit order for a position.
    pub async fn sell_position(
        &self,
        token_id: &str,
        quantity: Decimal,
        price: Decimal,
    ) -> Result<String> {
        let price = price.round_dp(3);
        if price == Decimal::ZERO {
            return Err(FalkeError::OrderRejected("price is zero".into()));
        }
        info!("LIVE SELL: {} qty={} @ {}", token_id, quantity, price);
        self.clob
            .place_order(token_id, OrderSide::Sell, price, quantity)
            .await
    }

    /// Fetch CLOB USDC balance.
    pub async fn clob_balance(&self) -> Option<Decimal> {
        self.clob.balance_usdc().await
    }
}

/// Returns true if this error indicates a balance/allowance problem that
/// won't resolve by retrying — the user needs to top up or approve.
pub fn is_balance_error(e: &crate::error::FalkeError) -> bool {
    let s = e.to_string();
    s.contains("not enough balance") || s.contains("allowance") || s.contains("min size")
}
