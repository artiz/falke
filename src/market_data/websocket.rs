use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use super::collector::SharedMarketData;

const POLYMARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

/// Optional WebSocket-based price streaming for lower latency updates.
/// Falls back to polling via the collector if WebSocket connection fails.
pub async fn run_websocket_stream(market_data: SharedMarketData) {
    loop {
        info!("Connecting to Polymarket WebSocket...");

        match connect_async(POLYMARKET_WS_URL).await {
            Ok((mut ws_stream, _)) => {
                info!("WebSocket connected to Polymarket");

                // Subscribe to market updates
                let subscribe_msg = serde_json::json!({
                    "type": "subscribe",
                    "channel": "market"
                });

                if let Err(e) = ws_stream
                    .send(Message::Text(subscribe_msg.to_string().into()))
                    .await
                {
                    error!("Failed to send subscribe message: {e}");
                    continue;
                }

                while let Some(msg) = ws_stream.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Err(e) =
                                handle_ws_message(&text, &market_data).await
                            {
                                debug!("Failed to handle WS message: {e}");
                            }
                        }
                        Ok(Message::Ping(data)) => {
                            if let Err(e) = ws_stream.send(Message::Pong(data)).await {
                                warn!("Failed to send pong: {e}");
                                break;
                            }
                        }
                        Ok(Message::Close(_)) => {
                            info!("WebSocket closed by server");
                            break;
                        }
                        Err(e) => {
                            error!("WebSocket error: {e}");
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                error!("Failed to connect WebSocket: {e}");
            }
        }

        // Reconnect after a delay
        warn!("WebSocket disconnected, reconnecting in 5 seconds...");
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

async fn handle_ws_message(
    text: &str,
    market_data: &SharedMarketData,
) -> Result<(), Box<dyn std::error::Error>> {
    let value: serde_json::Value = serde_json::from_str(text)?;

    // Extract price updates from the WebSocket message
    if let Some(market_id) = value.get("market").and_then(|m| m.as_str()) {
        if let Some(price) = value.get("price").and_then(|p| p.as_f64()) {
            if let Some(token_id) = value.get("asset_id").and_then(|t| t.as_str()) {
                let decimal_price =
                    rust_decimal::Decimal::from_str_exact(&format!("{price:.6}"))?;

                let mut data = market_data.write().await;
                data.price_store.add_price(token_id, decimal_price);

                debug!(
                    "WS price update: market={market_id} token={token_id} price={price}"
                );
            }
        }
    }

    Ok(())
}
