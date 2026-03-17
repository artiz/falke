use rust_decimal::Decimal;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared, runtime-mutable config
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
    pub trading_mode: TradingMode,
    pub paper_balance: Decimal,
    pub trading_paused: bool,

    // Telegram
    pub telegram_bot_token: String,
    pub allowed_phones: HashSet<String>,

    // Polymarket
    pub gamma_api_url: String,
    pub clob_api_url: String,

    // Wallet (for live trading)
    pub wallet_private_key: Option<String>,
    pub polygon_rpc_url: String,
    pub process_usdc_allowances: bool,

    // Tail Risk strategy
    pub tail_risk_max_price: Decimal,
    pub tail_risk_bet_usd: Decimal,
    pub tail_risk_kelly_edge_multiplier: f64,
    pub tail_risk_min_payout_multiplier: f64,
    pub tail_risk_take_profit_pct: Decimal,
    pub tail_risk_stop_loss_pct: Decimal,
    pub tail_risk_take_profit_fraction: f64,

    // Market filters
    pub market_expiry_window_hours: u32,
    pub min_liquidity_usd: Decimal,
    pub ignored_topics: Vec<String>,

    // Risk / engine
    pub trade_poll_interval_sec: u64,
    pub max_bet_usd: Decimal,
    pub max_open_positions: usize,
    pub cooldown_sec: u64,
    pub pnl_notify_threshold_usd: Decimal,
    /// Circuit breaker: pause trading if portfolio loss exceeds this % of initial balance (0 = disabled)
    pub budget_brake_pct: Decimal,
    /// How long to pause (seconds) when the budget brake fires
    pub budget_brake_time_sec: u64,

    // Strategy testing / parameter sweep
    pub testing_mode: bool,
    pub test_max_price_min: Decimal,
    pub test_max_price_max: Decimal,
    pub test_bet_usd_min: Decimal,
    pub test_bet_usd_max: Decimal,
    pub test_take_profit_pct_min: Decimal,
    pub test_take_profit_pct_max: Decimal,

    // AWS / DynamoDB
    pub aws_region: String,
    pub dynamo_table_prefix: String,
    pub dynamo_endpoint: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let _ = dotenvy::dotenv();

        let allowed_phones: HashSet<String> = std::env::var("ALLOWED_PHONES")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let wallet_key = std::env::var("WALLET_PRIVATE_KEY")
            .ok()
            .filter(|s| !s.is_empty());

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
            polygon_rpc_url: env_or("POLYGON_RPC_URL", "https://polygon-bor-rpc.publicnode.com"),
            process_usdc_allowances: std::env::var("PROCESS_USDC_ALLOWANCES")
                .unwrap_or_default()
                .to_lowercase()
                == "true",

            tail_risk_max_price: decimal_env("TAIL_RISK_MAX_PRICE", "0.03")?,
            tail_risk_bet_usd: decimal_env("TAIL_RISK_BET_USD", "5.0")?,
            tail_risk_kelly_edge_multiplier: env_or("TAIL_RISK_KELLY_EDGE_MULTIPLIER", "2.0")
                .parse()?,
            tail_risk_min_payout_multiplier: env_or("TAIL_RISK_MIN_PAYOUT_MULTIPLIER", "25.0")
                .parse()?,
            tail_risk_take_profit_pct: decimal_env("TAIL_RISK_TAKE_PROFIT_PCT", "50.0")?,
            tail_risk_stop_loss_pct: decimal_env("TAIL_RISK_STOP_LOSS_PCT", "0.0")?,
            tail_risk_take_profit_fraction: env_or("TAIL_RISK_TAKE_PROFIT_FRACTION", "0.5")
                .parse()?,

            market_expiry_window_hours: env_or("MARKET_EXPIRY_WINDOW_HOURS", "4").parse()?,
            min_liquidity_usd: decimal_env("MIN_LIQUIDITY_USD", "1000.0")?,
            ignored_topics: env_or("IGNORED_TOPICS", "politics")
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),

            trade_poll_interval_sec: env_or("TRADE_POLL_INTERVAL_SEC", "10").parse()?,
            max_bet_usd: decimal_env("MAX_BET_USD", "50.0")?,
            max_open_positions: env_or("MAX_OPEN_POSITIONS", "100").parse()?,
            cooldown_sec: env_or("COOLDOWN_SEC", "600").parse()?,
            pnl_notify_threshold_usd: decimal_env("PNL_NOTIFY_THRESHOLD_USD", "20.0")?,
            budget_brake_pct: decimal_env("BUDGET_BRAKE_PCT", "0.0")?,
            budget_brake_time_sec: env_or("BUDGET_BRAKE_TIME_SEC", "3600").parse()?,

            testing_mode: std::env::var("TESTING_MODE")
                .unwrap_or_default()
                .to_lowercase()
                == "true",
            test_max_price_min: decimal_env("TAIL_RISK_MAX_PRICE_MIN", "0.03")?,
            test_max_price_max: decimal_env("TAIL_RISK_MAX_PRICE_MAX", "0.03")?,
            test_bet_usd_min: decimal_env("TAIL_RISK_BET_USD_MIN", "5.0")?,
            test_bet_usd_max: decimal_env("TAIL_RISK_BET_USD_MAX", "5.0")?,
            test_take_profit_pct_min: decimal_env("TAIL_RISK_TAKE_PROFIT_PCT_MIN", "50.0")?,
            test_take_profit_pct_max: decimal_env("TAIL_RISK_TAKE_PROFIT_PCT_MAX", "50.0")?,

            aws_region: env_or("AWS_REGION", "eu-west-2"),
            dynamo_table_prefix: env_or("DYNAMO_TABLE_PREFIX", "falke"),
            dynamo_endpoint: std::env::var("DYNAMO_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }
}

impl Config {
    /// Merge DB-persisted settings on top of env-loaded values.
    /// Called once at startup after loading env config.
    pub fn apply_db_settings(&mut self, s: &crate::db::models::GlobalSettings) {
        self.trading_paused = s.paused;
        if let Some(v) = s.tail_risk_take_profit_pct {
            self.tail_risk_take_profit_pct = v;
        }
        if let Some(v) = s.tail_risk_bet_usd {
            self.tail_risk_bet_usd = v;
        }
        if let Some(v) = s.tail_risk_max_price {
            self.tail_risk_max_price = v;
        }
        if let Some(v) = s.market_expiry_window_hours {
            self.market_expiry_window_hours = v;
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn decimal_env(key: &str, default: &str) -> Result<Decimal, rust_decimal::Error> {
    Decimal::from_str(&env_or(key, default))
}
