use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use std::collections::HashMap;
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

    /// Fetch active markets expiring within the given number of hours
    pub async fn fetch_expiring_markets(&self, expiry_hours: f64) -> Result<Vec<GammaMarket>> {
        let now = Utc::now();
        let max_end = now + Duration::seconds((expiry_hours * 3600.0) as i64);

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
            "Fetched {} expiring markets (within {}h)",
            all_markets.len(),
            expiry_hours
        );

        Ok(all_markets)
    }

    /// Fetch current prices for markets that have dropped out of the tracking window.
    /// Returns a map of condition_id → list of (token_id, price).
    ///
    /// Tries two passes:
    ///   1. Recently closed/resolved markets (closed=true, last 30 days)
    ///   2. Active markets within a wide window (up to 30 days ahead) — catches positions
    ///      entered when the tracking window was wider than the current setting.
    pub async fn fetch_prices_for_stale(
        &self,
        condition_ids: &std::collections::HashSet<String>,
    ) -> HashMap<String, Vec<(String, Decimal)>> {
        let mut result: HashMap<String, Vec<(String, Decimal)>> = HashMap::new();

        // Pass 1: closed/resolved markets
        self.fetch_market_pages(
            &[
                ("closed", "true"),
                ("end_date_min", &(Utc::now() - Duration::days(30)).format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                ("end_date_max", &(Utc::now() + Duration::hours(1)).format("%Y-%m-%dT%H:%M:%SZ").to_string()),
            ],
            condition_ids,
            &mut result,
        )
        .await;

        // Pass 2: active markets with wide window (up to 30 days ahead)
        if result.len() < condition_ids.len() {
            self.fetch_market_pages(
                &[
                    ("active", "true"),
                    ("closed", "false"),
                    ("end_date_min", &Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                    ("end_date_max", &(Utc::now() + Duration::days(30)).format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                ],
                condition_ids,
                &mut result,
            )
            .await;
        }

        result
    }

    /// Internal helper: paginate /markets with given query params, collect prices for condition_ids we care about.
    async fn fetch_market_pages(
        &self,
        params: &[(&str, &str)],
        condition_ids: &std::collections::HashSet<String>,
        result: &mut HashMap<String, Vec<(String, Decimal)>>,
    ) {
        let mut offset = 0usize;
        let limit = 500usize;
        let limit_str = limit.to_string();

        loop {
            let offset_str = offset.to_string();
            let mut query: Vec<(&str, &str)> = params.to_vec();
            query.push(("limit", &limit_str));
            query.push(("offset", &offset_str));

            let resp = self
                .client
                .get(format!("{}/markets", self.base_url))
                .query(&query)
                .send()
                .await;

            let resp = match resp {
                Ok(r) if r.status().is_success() => r,
                _ => break,
            };

            let markets: Vec<GammaMarket> = match resp.json().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let count = markets.len();

            for m in &markets {
                if !condition_ids.contains(&m.condition_id) || result.contains_key(&m.condition_id) {
                    continue;
                }
                let prices = m.parsed_prices();
                let token_ids = m.parsed_token_ids();
                let entries: Vec<(String, Decimal)> = if m.closed && !prices.is_empty() {
                    // Resolved market: force winner → 1.0, loser → 0.0
                    let max_price = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                    token_ids
                        .into_iter()
                        .zip(prices.iter())
                        .map(|(tid, p)| {
                            let v = if (*p - max_price).abs() < 1e-6 { Decimal::ONE } else { Decimal::ZERO };
                            (tid, v)
                        })
                        .collect()
                } else {
                    token_ids
                        .into_iter()
                        .zip(prices.iter())
                        .map(|(tid, p)| {
                            (tid, Decimal::from_str(&format!("{p:.6}")).unwrap_or(Decimal::ZERO))
                        })
                        .collect()
                };
                if !entries.is_empty() {
                    result.insert(m.condition_id.clone(), entries);
                }
            }

            if result.len() == condition_ids.len() || count < limit {
                break;
            }
            offset += limit;
        }
    }

    /// Fetch all condition_ids belonging to events with any of the given tag slugs.
    pub async fn fetch_ignored_condition_ids(
        &self,
        tag_slugs: &[String],
    ) -> std::collections::HashSet<String> {
        let mut ignored = std::collections::HashSet::new();
        for tag in tag_slugs {
            let mut offset = 0usize;
            loop {
                let resp = self
                    .client
                    .get(format!("{}/events", self.base_url))
                    .query(&[
                        ("active", "true"),
                        ("closed", "false"),
                        ("tag_slug", tag.as_str()),
                        ("limit", "500"),
                        ("offset", &offset.to_string()),
                    ])
                    .send()
                    .await;
                let resp = match resp {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Failed to fetch ignored events for tag {tag}: {e}");
                        break;
                    }
                };
                if !resp.status().is_success() {
                    break;
                }
                let events: Vec<serde_json::Value> = match resp.json().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let count = events.len();
                for event in &events {
                    if let Some(markets) = event.get("markets").and_then(|m| m.as_array()) {
                        for market in markets {
                            if let Some(cid) = market.get("conditionId").and_then(|v| v.as_str()) {
                                ignored.insert(cid.to_string());
                            }
                        }
                    }
                }
                if count < 500 {
                    break;
                }
                offset += count;
            }
        }
        ignored
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

        let group_slug = market.events.first().and_then(|e| e.slug.clone());

        Some(TrackedMarket {
            condition_id: market.condition_id.clone(),
            question: market.question.clone(),
            end_date,
            outcomes,
            liquidity,
            last_updated: Utc::now(),
            slug: market.slug.clone(),
            group_slug,
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
