#![allow(dead_code)]
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::{HashMap, VecDeque};

/// A single price data point
#[derive(Debug, Clone)]
pub struct PricePoint {
    pub price: Decimal,
    pub timestamp: DateTime<Utc>,
}

/// Derivative calculation result
#[derive(Debug, Clone)]
pub struct DerivativeResult {
    /// Smoothed price change per second (via linear regression)
    pub derivative_per_sec: f64,
    /// Total percentage change over the window
    pub pct_change: f64,
    /// Raw percentage change (start vs end, no smoothing)
    pub pct_change_raw: f64,
    /// Number of data points in the window
    pub num_points: usize,
    /// First price in window
    pub start_price: Decimal,
    /// Last price in window
    pub end_price: Decimal,
    /// Actual window duration in seconds
    pub window_sec: f64,
}

/// In-memory ring buffer price store for all tracked tokens.
///
/// Memory estimate: 1000 tokens x 50 points x ~64 bytes = ~3.2 MB
pub struct PriceStore {
    /// token_id -> ring buffer of price points
    data: HashMap<String, VecDeque<PricePoint>>,
    /// Max data points per token
    max_points: usize,
    /// Window for derivative calculations (seconds)
    window_sec: u64,
    /// Minimum points before we can calculate a derivative
    min_points: usize,
}

impl PriceStore {
    pub fn new(max_points: usize, window_sec: u64, min_points: usize) -> Self {
        Self {
            data: HashMap::new(),
            max_points,
            window_sec,
            min_points,
        }
    }

    /// Record a new price for a token
    pub fn add_price(&mut self, token_id: &str, price: Decimal) {
        let buffer = self
            .data
            .entry(token_id.to_string())
            .or_insert_with(|| VecDeque::with_capacity(self.max_points));

        if buffer.len() >= self.max_points {
            buffer.pop_front();
        }

        buffer.push_back(PricePoint {
            price,
            timestamp: Utc::now(),
        });
    }

    /// Get recent price points within the configured window
    pub fn get_recent(&self, token_id: &str) -> Vec<&PricePoint> {
        let cutoff = Utc::now() - chrono::Duration::seconds(self.window_sec as i64);

        match self.data.get(token_id) {
            Some(buffer) => buffer.iter().filter(|p| p.timestamp >= cutoff).collect(),
            None => Vec::new(),
        }
    }

    /// Compute the derivative (rate of change) over the configured window.
    /// Uses linear regression for smoothed results.
    pub fn compute_derivative(&self, token_id: &str) -> Option<DerivativeResult> {
        let points = self.get_recent(token_id);

        if points.len() < self.min_points {
            return None;
        }

        let start = points.first()?;
        let end = points.last()?;

        let dt = (end.timestamp - start.timestamp).num_milliseconds() as f64 / 1000.0;
        if dt < 30.0 {
            return None; // Need at least 30 seconds
        }

        let start_f = decimal_to_f64(start.price);
        let end_f = decimal_to_f64(end.price);

        if start_f == 0.0 {
            return None;
        }

        let raw_pct = (end_f - start_f) / start_f;

        // Linear regression: price = slope * time + intercept
        let base_time = start.timestamp.timestamp_millis() as f64 / 1000.0;
        let n = points.len() as f64;

        let mut sum_t = 0.0;
        let mut sum_p = 0.0;
        let mut sum_tp = 0.0;
        let mut sum_tt = 0.0;

        for point in &points {
            let t = (point.timestamp.timestamp_millis() as f64 / 1000.0) - base_time;
            let p = decimal_to_f64(point.price);
            sum_t += t;
            sum_p += p;
            sum_tp += t * p;
            sum_tt += t * t;
        }

        let denominator = n * sum_tt - sum_t * sum_t;
        let (slope, pct_change) = if denominator.abs() > 1e-10 {
            let s = (n * sum_tp - sum_t * sum_p) / denominator;
            let pct = (s * dt) / start_f;
            (s, pct)
        } else {
            let s = (end_f - start_f) / dt;
            (s, raw_pct)
        };

        Some(DerivativeResult {
            derivative_per_sec: slope,
            pct_change,
            pct_change_raw: raw_pct,
            num_points: points.len(),
            start_price: start.price,
            end_price: end.price,
            window_sec: dt,
        })
    }

    /// Get storage statistics
    pub fn stats(&self) -> StoreStats {
        let total_points: usize = self.data.values().map(|b| b.len()).sum();
        StoreStats {
            num_tokens_tracked: self.data.len(),
            total_data_points: total_points,
            approx_memory_bytes: total_points * 64,
        }
    }

    /// Remove all data for tokens not in the provided set (cleanup stale data)
    pub fn retain_tokens(&mut self, active_token_ids: &std::collections::HashSet<String>) {
        self.data.retain(|id, _| active_token_ids.contains(id));
    }
}

#[derive(Debug)]
pub struct StoreStats {
    pub num_tokens_tracked: usize,
    pub total_data_points: usize,
    pub approx_memory_bytes: usize,
}

fn decimal_to_f64(d: Decimal) -> f64 {
    use std::str::FromStr;
    f64::from_str(&d.to_string()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_add_and_retrieve_prices() {
        let mut store = PriceStore::new(100, 300, 3);
        store.add_price("token1", dec!(0.50));
        store.add_price("token1", dec!(0.52));
        store.add_price("token1", dec!(0.55));

        let recent = store.get_recent("token1");
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_ring_buffer_overflow() {
        let mut store = PriceStore::new(3, 300, 2);
        store.add_price("t", dec!(0.1));
        store.add_price("t", dec!(0.2));
        store.add_price("t", dec!(0.3));
        store.add_price("t", dec!(0.4));

        let stats = store.stats();
        assert_eq!(stats.total_data_points, 3);
    }

    #[test]
    fn test_derivative_insufficient_points() {
        let mut store = PriceStore::new(100, 300, 5);
        store.add_price("t", dec!(0.50));
        store.add_price("t", dec!(0.55));

        assert!(store.compute_derivative("t").is_none());
    }
}
