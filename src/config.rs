#![allow(dead_code)]
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingMode {
    Paper,
    Live,
}

impl FromStr for TradingMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "paper" => Ok(TradingMode::Paper),
            "live" => Ok(TradingMode::Live),
            _ => Err(format!("Invalid trading mode: {s}. Use 'paper' or 'live'.")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    // Trading mode
    pub trading_mode: TradingMode,
    pub paper_balance: Decimal,

    // Telegram
    pub telegram_bot_token: String,
    pub allowed_phones: HashSet<String>,

    // Polymarket
    pub gamma_api_url: String,
    pub clob_api_url: String,

    // Wallet
    pub wallet_private_key: Option<String>,

    // Strategy — Arbitrage
    pub arb_threshold: Decimal,
    pub arb_budget_pct: Decimal,

    // Strategy — Momentum
    pub momentum_derivative_threshold: Decimal,
    pub momentum_window_sec: u64,
    pub momentum_budget_pct: Decimal,
    pub momentum_poll_interval_sec: u64,

    // Market filters
    pub market_expiry_window_days: u32,
    pub min_liquidity_usd: Decimal,

    // Risk management
    pub max_bet_usd: Decimal,
    pub max_open_positions: usize,
    pub cooldown_sec: u64,

    // AWS / DynamoDB
    pub aws_region: String,
    pub dynamo_table_prefix: String,
    /// Custom endpoint for local DynamoDB (e.g. LocalStack). None = use real AWS.
    pub dynamo_endpoint: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let _ = dotenvy::dotenv(); // ignore if .env not found

        let allowed_phones: HashSet<String> = std::env::var("ALLOWED_PHONES")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let wallet_key = std::env::var("WALLET_PRIVATE_KEY").ok().filter(|s| !s.is_empty());

        Ok(Config {
            trading_mode: env_or("TRADING_MODE", "paper").parse()?,
            paper_balance: decimal_env("PAPER_BALANCE", "1000.0")?,

            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN")
                .map_err(|_| "TELEGRAM_BOT_TOKEN is required")?,
            allowed_phones,

            gamma_api_url: env_or("GAMMA_API_URL", "https://gamma-api.polymarket.com"),
            clob_api_url: env_or("CLOB_API_URL", "https://clob.polymarket.com"),

            wallet_private_key: wallet_key,

            arb_threshold: decimal_env("ARB_THRESHOLD", "0.97")?,
            arb_budget_pct: decimal_env("ARB_BUDGET_PCT", "0.50")?,

            momentum_derivative_threshold: decimal_env("MOMENTUM_DERIVATIVE_THRESHOLD", "0.30")?,
            momentum_window_sec: env_or("MOMENTUM_WINDOW_SEC", "300").parse()?,
            momentum_budget_pct: decimal_env("MOMENTUM_BUDGET_PCT", "0.50")?,
            momentum_poll_interval_sec: env_or("MOMENTUM_POLL_INTERVAL_SEC", "10").parse()?,

            market_expiry_window_days: env_or("MARKET_EXPIRY_WINDOW_DAYS", "3").parse()?,
            min_liquidity_usd: decimal_env("MIN_LIQUIDITY_USD", "1000.0")?,

            max_bet_usd: decimal_env("MAX_BET_USD", "50.0")?,
            max_open_positions: env_or("MAX_OPEN_POSITIONS", "20").parse()?,
            cooldown_sec: env_or("COOLDOWN_SEC", "600").parse()?,

            aws_region: env_or("AWS_REGION", "eu-west-2"),
            dynamo_table_prefix: env_or("DYNAMO_TABLE_PREFIX", "falke"),
            dynamo_endpoint: std::env::var("DYNAMO_ENDPOINT").ok().filter(|s| !s.is_empty()),
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn decimal_env(key: &str, default: &str) -> Result<Decimal, rust_decimal::Error> {
    let val = env_or(key, default);
    Decimal::from_str(&val)
}
