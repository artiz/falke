use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use reqwest::Client;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, trace};

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

        let end_date_min = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let end_date_max = max_end.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let mut all_markets = Vec::new();
        let mut offset = 0;
        let limit: usize = 100;

        loop {
            let resp = self
                .client
                .get(format!("{}/markets", self.base_url))
                .query(&[
                    ("active", "true"),
                    ("closed", "false"),
                    ("end_date_min", end_date_min.as_str()),
                    ("end_date_max", end_date_max.as_str()),
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
                if market.active && !market.closed {
                    trace!(
                        "Including market: {} (ends {:?})",
                        market.question,
                        market.end_date
                    );
                    all_markets.push(market);
                }
            }

            if count < limit {
                break;
            }
            offset += limit;
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
        let end_date = parse_end_date(&market.end_date);

        let outcome_names = market.parsed_outcomes();
        let prices = market.parsed_prices();
        let token_ids = market.parsed_token_ids();

        // Need at least outcomes and prices
        if outcome_names.is_empty() || prices.is_empty() {
            return None;
        }

        let outcomes: Vec<TrackedOutcome> = outcome_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let price = prices.get(i).copied().unwrap_or(0.0);
                let token_id = token_ids
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("{}_{}", market.condition_id, i));

                TrackedOutcome {
                    token_id,
                    name: name.clone(),
                    price: Decimal::from_str(&format!("{price:.6}")).unwrap_or(Decimal::ZERO),
                }
            })
            .collect();

        if outcomes.is_empty() {
            return None;
        }

        let liquidity =
            Decimal::from_str(&format!("{:.2}", market.liquidity_f64())).unwrap_or(Decimal::ZERO);

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

/// Parse various date formats the Gamma API may return
fn parse_end_date(date_str: &Option<String>) -> Option<DateTime<Utc>> {
    let s = date_str.as_ref()?;

    // Try RFC3339 first: "2026-03-31T12:00:00Z"
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }

    // Try without timezone: "2026-03-31T12:00:00"
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.and_utc());
    }

    // Try date only: "2026-03-31"
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return dt.and_hms_opt(23, 59, 59).map(|ndt| ndt.and_utc());
    }

    None
}
