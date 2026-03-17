use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A registered bot user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Telegram user ID (primary key)
    pub telegram_id: i64,
    /// Telegram username (optional)
    pub username: Option<String>,
    /// Verified phone number
    pub phone_number: String,
    /// Linked Polymarket wallet address (optional until linked)
    pub wallet_address: Option<String>,
    /// Trading mode: "paper" or "live"
    pub trading_mode: String,
    /// Whether the bot is actively trading for this user
    pub is_active: bool,
    /// Registration timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_active: DateTime<Utc>,
}

/// Persisted global bot settings (single row, key = "global")
#[derive(Debug, Clone, Default)]
pub struct GlobalSettings {
    pub paused: bool,
    pub trading_mode: Option<String>,
    pub tail_risk_take_profit_pct: Option<rust_decimal::Decimal>,
    pub tail_risk_bet_usd: Option<rust_decimal::Decimal>,
    pub tail_risk_max_price: Option<rust_decimal::Decimal>,
    pub market_expiry_window_hours: Option<u32>,
}

/// Stored trade record (for persistence beyond in-memory portfolio)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    /// Unique trade ID (UUID)
    pub trade_id: String,
    /// User's telegram ID
    pub user_id: i64,
    /// Market condition ID
    pub condition_id: String,
    /// Token ID
    pub token_id: String,
    /// Market question
    pub question: String,
    /// Outcome name
    pub outcome_name: String,
    /// "YES", "NO", or "ARB"
    pub side: String,
    /// Entry price
    pub entry_price: String,
    /// Exit price (None if still open)
    pub exit_price: Option<String>,
    /// Number of shares
    pub quantity: String,
    /// USD cost
    pub cost_basis: String,
    /// Realized P&L (None if still open)
    pub realized_pnl: Option<String>,
    /// "arbitrage" or "momentum"
    pub strategy: String,
    /// "paper" or "live"
    pub mode: String,
    /// "open", "closed", "expired"
    pub status: String,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}
