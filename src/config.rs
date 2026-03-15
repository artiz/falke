#![allow(dead_code)]
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared, runtime-mutable config (e.g. for strategy hot-swapping via Telegram)
pub type SharedConfig = Arc<RwLock<Config>>;

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
    pub trading_paused: bool,

    // Telegram
    pub telegram_bot_token: String,
    pub allowed_phones: HashSet<String>,

    // Polymarket
    pub gamma_api_url: String,
    pub clob_api_url: String,

    // Wallet
    pub wallet_private_key: Option<String>,

    // Polymarket Relayer API credentials (from Polymarket UI → Profile → API Keys)
    pub relayer_api_key: Option<String>,
    pub relayer_api_key_address: Option<String>,

    // Strategy — Arbitrage
    pub arb_threshold: Decimal,
    pub arb_budget_pct: Decimal,

    // Strategy — Momentum
    pub momentum_derivative_threshold: Decimal,
    pub momentum_window_sec: u64,
    pub momentum_budget_pct: Decimal,
    pub trade_poll_interval_sec: u64,

    // Strategy — Mean Reversion
    pub mean_reversion_threshold: Decimal,
    pub mean_reversion_budget_pct: Decimal,

    // Strategy — Tail Risk (buy cheap long-shot outcomes)
    pub tail_risk_max_price: Decimal,
    pub tail_risk_budget_pct: Decimal,
    pub tail_risk_bet_usd: Decimal,
    /// How much we think the market underprices tail events (e.g. 2.0 = true prob is 2x market price)
    pub tail_risk_kelly_edge_multiplier: f64,

    // Market filters
    pub market_expiry_window_days: u32,
    pub min_liquidity_usd: Decimal,

    // Risk management
    pub max_bet_usd: Decimal,
    pub max_open_positions: usize,
    pub cooldown_sec: u64,
    pub take_profit_pct: Decimal,
    pub stop_loss_pct: Decimal,
    pub tail_risk_take_profit_pct: Decimal,
    pub tail_risk_stop_loss_pct: Decimal,
    /// Fraction of tail risk positions that use TP exit (rest hold to resolution)
    pub tail_risk_take_profit_fraction: f64,
    pub pnl_notify_threshold_usd: Decimal,

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
            trading_paused: false,
            paper_balance: decimal_env("PAPER_BALANCE", "1000.0")?,

            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN")
                .map_err(|_| "TELEGRAM_BOT_TOKEN is required")?,
            allowed_phones,

            gamma_api_url: env_or("GAMMA_API_URL", "https://gamma-api.polymarket.com"),
            clob_api_url: env_or("CLOB_API_URL", "https://clob.polymarket.com"),

            wallet_private_key: wallet_key,

            relayer_api_key: std::env::var("RELAYER_API_KEY").ok().filter(|s| !s.is_empty()),
            relayer_api_key_address: std::env::var("RELAYER_API_KEY_ADDRESS").ok().filter(|s| !s.is_empty()),

            arb_threshold: decimal_env("ARB_THRESHOLD", "0.97")?,
            arb_budget_pct: decimal_env("ARB_BUDGET_PCT", "0.00")?,

            momentum_derivative_threshold: decimal_env("MOMENTUM_DERIVATIVE_THRESHOLD", "0.30")?,
            momentum_window_sec: env_or("MOMENTUM_WINDOW_SEC", "300").parse()?,
            momentum_budget_pct: decimal_env("MOMENTUM_BUDGET_PCT", "0.00")?,
            trade_poll_interval_sec: env_or("TRADE_POLL_INTERVAL_SEC", "10").parse()?,

            mean_reversion_threshold: decimal_env("MEAN_REVERSION_THRESHOLD", "0.20")?,
            mean_reversion_budget_pct: decimal_env("MEAN_REVERSION_BUDGET_PCT", "0.00")?,

            tail_risk_max_price: decimal_env("TAIL_RISK_MAX_PRICE", "0.05")?,
            tail_risk_budget_pct: decimal_env("TAIL_RISK_BUDGET_PCT", "1.00")?,
            tail_risk_bet_usd: decimal_env("TAIL_RISK_BET_USD", "5.0")?,
            tail_risk_kelly_edge_multiplier: env_or("TAIL_RISK_KELLY_EDGE_MULTIPLIER", "2.0").parse()?,

            market_expiry_window_days: env_or("MARKET_EXPIRY_WINDOW_DAYS", "3").parse()?,
            min_liquidity_usd: decimal_env("MIN_LIQUIDITY_USD", "1000.0")?,

            max_bet_usd: decimal_env("MAX_BET_USD", "50.0")?,
            max_open_positions: env_or("MAX_OPEN_POSITIONS", "20").parse()?,
            cooldown_sec: env_or("COOLDOWN_SEC", "600").parse()?,
            take_profit_pct: decimal_env("TAKE_PROFIT_PCT", "10.0")?,
            tail_risk_take_profit_pct: decimal_env("TAIL_RISK_TAKE_PROFIT_PCT", "30.0")?,
            tail_risk_stop_loss_pct: decimal_env("TAIL_RISK_STOP_LOSS_PCT", "8.0")?,
            
            tail_risk_take_profit_fraction: env_or("TAIL_RISK_TAKE_PROFIT_FRACTION", "0.5").parse()?,
            stop_loss_pct: decimal_env("STOP_LOSS_PCT", "8.0")?,
            pnl_notify_threshold_usd: decimal_env("PNL_NOTIFY_THRESHOLD_USD", "20.0")?,

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
