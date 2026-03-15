#![allow(dead_code)]
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A market from the Gamma API (matches actual API response format)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GammaMarket {
    /// Unique condition ID
    pub condition_id: String,

    /// Human-readable question
    pub question: String,

    /// Market description
    #[serde(default)]
    pub description: String,

    /// End date (ISO 8601) e.g. "2026-03-31T12:00:00Z"
    #[serde(default)]
    pub end_date: Option<String>,

    /// Whether the market is active
    #[serde(default)]
    pub active: bool,

    /// Whether the market is closed
    #[serde(default)]
    pub closed: bool,

    /// Liquidity as string e.g. "1009.33"
    #[serde(default)]
    pub liquidity: Option<String>,

    /// Liquidity as number
    #[serde(default)]
    pub liquidity_num: Option<f64>,

    /// Volume as string
    #[serde(default)]
    pub volume: Option<String>,

    /// Market slug
    #[serde(default)]
    pub slug: Option<String>,

    /// Outcomes as JSON string: "[\"Yes\", \"No\"]"
    #[serde(default)]
    pub outcomes: Option<String>,

    /// Outcome prices as JSON string: "[\"0.262\", \"0.738\"]"
    #[serde(default)]
    pub outcome_prices: Option<String>,

    /// CLOB token IDs as JSON string
    #[serde(default)]
    pub clob_token_ids: Option<String>,

    /// Whether the order book is enabled
    #[serde(default)]
    pub enable_order_book: Option<bool>,

    /// Whether the market is accepting orders
    #[serde(default)]
    pub accepting_orders: Option<bool>,
}

impl GammaMarket {
    /// Parse the outcomes JSON string into a Vec<String>
    pub fn parsed_outcomes(&self) -> Vec<String> {
        self.outcomes
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }

    /// Parse the outcome prices JSON string into a Vec<f64>
    pub fn parsed_prices(&self) -> Vec<f64> {
        self.outcome_prices
            .as_ref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .map(|v| v.iter().filter_map(|p| p.parse::<f64>().ok()).collect())
            .unwrap_or_default()
    }

    /// Parse the CLOB token IDs JSON string into a Vec<String>
    pub fn parsed_token_ids(&self) -> Vec<String> {
        self.clob_token_ids
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }

    /// Get liquidity as f64
    pub fn liquidity_f64(&self) -> f64 {
        self.liquidity_num.unwrap_or_else(|| {
            self.liquidity
                .as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0)
        })
    }
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
