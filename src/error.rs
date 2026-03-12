use thiserror::Error;

#[derive(Error, Debug)]
pub enum FalkeError {
    // API errors
    #[error("Polymarket API error: {0}")]
    PolymarketApi(String),

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    // Trading errors
    #[error("Insufficient balance: need {needed}, have {available}")]
    InsufficientBalance {
        needed: String,
        available: String,
    },

    #[error("Market not found: {0}")]
    MarketNotFound(String),

    #[error("Order rejected: {0}")]
    OrderRejected(String),

    #[error("Position limit reached: max {0} open positions")]
    PositionLimitReached(usize),

    #[error("Market in cooldown: {market_id}, {seconds_remaining}s remaining")]
    MarketCooldown {
        market_id: String,
        seconds_remaining: u64,
    },

    // Auth errors
    #[error("Unauthorized phone number: {0}")]
    UnauthorizedPhone(String),

    #[error("User not registered: telegram_id={0}")]
    UserNotRegistered(i64),

    #[error("Polymarket account not linked for user: {0}")]
    AccountNotLinked(i64),

    #[error("Wallet error: {0}")]
    Wallet(String),

    // Database errors
    #[error("DynamoDB error: {0}")]
    DynamoDb(String),

    // Config errors
    #[error("Configuration error: {0}")]
    Config(String),

    // WebSocket errors
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    // Internal
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, FalkeError>;
