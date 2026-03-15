use rust_decimal::Decimal;
use tracing::info;

use crate::error::{FalkeError, Result};
use crate::polymarket::clob_api::{ClobClient, OrderRequest, OrderSide, OrderType};
use crate::strategy::signals::{Signal, SignalDirection};

/// Live trading executor — places real orders via the Polymarket CLOB API.
pub struct LiveExecutor {
    clob: ClobClient,
}

impl LiveExecutor {
    pub fn new(clob: ClobClient) -> Self {
        Self { clob }
    }

    /// Place a market order for a given signal
    pub async fn execute_signal(&self, signal: &Signal, amount_usd: Decimal) -> Result<String> {
        let side = match signal.direction {
            SignalDirection::BuyYes => OrderSide::Buy,
            SignalDirection::BuyNo => OrderSide::Buy, // Buy the NO token
        };

        let order = OrderRequest {
            token_id: signal.token_id.clone(),
            side,
            price: signal.current_price.to_string(),
            size: amount_usd.to_string(),
            order_type: OrderType::Fok, // Fill or kill for immediate execution
        };

        info!(
            "LIVE ORDER: {:?} {} @ {} | size=${}",
            order.side, signal.outcome_name, signal.current_price, amount_usd
        );

        let response = self.clob.place_order(&order).await?;

        match response.order_id {
            Some(id) => {
                info!("Order placed: {id}");
                Ok(id)
            }
            None => Err(FalkeError::OrderRejected(
                response
                    .error_msg
                    .unwrap_or_else(|| "Unknown error".to_string()),
            )),
        }
    }
}
