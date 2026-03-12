use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::types::OrderBook;
use crate::error::{FalkeError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct OrderRequest {
    pub token_id: String,
    pub side: OrderSide,
    pub price: String,
    pub size: String,
    pub order_type: OrderType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    Gtc, // Good till cancelled
    Fok, // Fill or kill
    Ioc, // Immediate or cancel
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderResponse {
    pub order_id: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub error_msg: Option<String>,
}

pub struct ClobClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
    api_secret: Option<String>,
    api_passphrase: Option<String>,
}

impl ClobClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("Failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: None,
            api_secret: None,
            api_passphrase: None,
        }
    }

    pub fn with_credentials(mut self, key: String, secret: String, passphrase: String) -> Self {
        self.api_key = Some(key);
        self.api_secret = Some(secret);
        self.api_passphrase = Some(passphrase);
        self
    }

    /// Fetch the order book for a given token
    pub async fn get_order_book(&self, token_id: &str) -> Result<OrderBook> {
        let resp = self
            .client
            .get(format!("{}/book", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(FalkeError::PolymarketApi(format!(
                "CLOB book API returned {status}: {body}"
            )));
        }

        let book: OrderBook = resp.json().await?;
        debug!("Fetched order book for token {token_id}: {} bids, {} asks", book.bids.len(), book.asks.len());
        Ok(book)
    }

    /// Place an order (requires authentication)
    pub async fn place_order(&self, order: &OrderRequest) -> Result<OrderResponse> {
        let (api_key, api_secret, api_passphrase) = match (
            &self.api_key,
            &self.api_secret,
            &self.api_passphrase,
        ) {
            (Some(k), Some(s), Some(p)) => (k, s, p),
            _ => {
                return Err(FalkeError::Wallet(
                    "CLOB API credentials not set. Derive them from wallet first.".into(),
                ))
            }
        };

        let resp = self
            .client
            .post(format!("{}/order", self.base_url))
            .header("POLY_API_KEY", api_key.as_str())
            .header("POLY_API_SECRET", api_secret.as_str())
            .header("POLY_PASSPHRASE", api_passphrase.as_str())
            .json(order)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(FalkeError::OrderRejected(format!(
                "CLOB order API returned {status}: {body}"
            )));
        }

        let response: OrderResponse = resp.json().await?;
        if let Some(ref err) = response.error_msg {
            return Err(FalkeError::OrderRejected(err.clone()));
        }

        Ok(response)
    }
}
