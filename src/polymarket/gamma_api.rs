use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, warn};

use super::types::{GammaMarket, TrackedMarket, TrackedOutcome};
use crate::error::{FalkeError, Result};

pub struct GammaClient {
    client: Client,
    base_url: String,
}

impl GammaClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Failed to build HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch active markets expiring within the given number of days
    pub async fn fetch_expiring_markets(&self, expiry_days: u32) -> Result<Vec<GammaMarket>> {
        let now = Utc::now();
        let max_end = now + Duration::days(expiry_days as i64);

        let mut all_markets = Vec::new();
        let mut offset = 0;
        let limit = 100;

        loop {
            let resp = self
                .client
                .get(format!("{}/markets", self.base_url))
                .query(&[
                    ("active", "true"),
                    ("closed", "false"),
                    ("limit", &limit.to_string()),
                    ("offset", &offset.to_string()),
                ])
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(FalkeError::PolymarketApi(format!(
                    "Gamma API returned {status}: {body}"
                )));
            }

            let markets: Vec<GammaMarket> = resp.json().await?;
            let count = markets.len();

            for market in markets {
                if let Some(ref end_str) = market.end_date_iso {
                    if let Ok(end_date) = DateTime::parse_from_rfc3339(end_str) {
                        let end_utc = end_date.with_timezone(&Utc);
                        if end_utc > now && end_utc <= max_end && market.active && !market.closed {
                            all_markets.push(market);
                        }
                    }
                }
            }

            if count < limit {
                break;
            }
            offset += limit;

            // Safety limit
            if offset > 1000 {
                warn!("Gamma API pagination limit reached at offset {offset}");
                break;
            }
        }

        debug!(
            "Fetched {} expiring markets (within {} days)",
            all_markets.len(),
            expiry_days
        );

        Ok(all_markets)
    }

    /// Convert a GammaMarket to our internal TrackedMarket format
    pub fn to_tracked_market(market: &GammaMarket) -> Option<TrackedMarket> {
        let end_date = market.end_date_iso.as_ref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        });

        let outcomes: Vec<TrackedOutcome> = market
            .tokens
            .iter()
            .map(|t| TrackedOutcome {
                token_id: t.token_id.clone(),
                name: t.outcome.clone(),
                price: Decimal::from_str(&t.price.unwrap_or(0.0).to_string())
                    .unwrap_or(Decimal::ZERO),
            })
            .collect();

        if outcomes.is_empty() {
            return None;
        }

        let liquidity = Decimal::from_str(&market.liquidity.unwrap_or(0.0).to_string())
            .unwrap_or(Decimal::ZERO);

        Some(TrackedMarket {
            condition_id: market.condition_id.clone(),
            question: market.question.clone(),
            end_date,
            outcomes,
            liquidity,
            last_updated: Utc::now(),
        })
    }
}
