use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A market from the Gamma API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaMarket {
    /// Unique condition ID
    #[serde(rename = "condition_id")]
    pub condition_id: String,

    /// Human-readable question
    pub question: String,

    /// Market description
    #[serde(default)]
    pub description: String,

    /// End date (ISO 8601)
    pub end_date_iso: Option<String>,

    /// Whether the market is active
    #[serde(default)]
    pub active: bool,

    /// Whether the market is closed
    #[serde(default)]
    pub closed: bool,

    /// Market outcomes/tokens
    #[serde(default)]
    pub tokens: Vec<GammaToken>,

    /// Total liquidity
    #[serde(default)]
    pub liquidity: Option<f64>,

    /// Total volume
    #[serde(default)]
    pub volume: Option<f64>,

    /// Market slug for URL construction
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaToken {
    /// Token ID used for trading on CLOB
    pub token_id: String,

    /// Outcome name (e.g., "Yes", "No", candidate name)
    pub outcome: String,

    /// Current price (0.0 to 1.0)
    #[serde(default)]
    pub price: Option<f64>,

    /// Winner status
    #[serde(default)]
    pub winner: bool,
}

/// Order book entry from CLOB API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookEntry {
    pub price: String,
    pub size: String,
}

/// Order book from CLOB API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub bids: Vec<OrderBookEntry>,
    pub asks: Vec<OrderBookEntry>,
    #[serde(default)]
    pub hash: Option<String>,
}

/// CLOB market info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClobMarket {
    pub condition_id: String,
    pub tokens: Vec<ClobToken>,
    #[serde(default)]
    pub min_tick_size: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClobToken {
    pub token_id: String,
    pub outcome: String,
    #[serde(default)]
    pub price: Option<f64>,
}

/// Internal representation of a market we're tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedMarket {
    pub condition_id: String,
    pub question: String,
    pub end_date: Option<DateTime<Utc>>,
    pub outcomes: Vec<TrackedOutcome>,
    pub liquidity: Decimal,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOutcome {
    pub token_id: String,
    pub name: String,
    pub price: Decimal,
}

/// A price snapshot for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub token_id: String,
    pub price: Decimal,
    pub timestamp: DateTime<Utc>,
}
