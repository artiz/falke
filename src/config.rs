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

    // Mean Reversion strategy
    pub mean_reversion_threshold: Decimal,
    /// Fraction of budget allocated to MR (0 = disabled, 1.0 = 100% MR)
    pub mean_reversion_budget_pct: Decimal,

    // ML signal filter (XGBoost ONNX model)
    /// Path to the ONNX model file (empty string = ML disabled)
    pub ml_model_path: String,
    /// Minimum win probability to accept a ML-filtered signal (0.0–1.0)
    pub ml_win_prob_threshold: f64,
    /// Minimum price % change pre-filter for ML scan (e.g. 0.10 = 10%)
    pub ml_reversion_threshold: Decimal,

    // Per-strategy bet sizes
    pub ml_bet_usd: Decimal,
    pub mr_bet_usd: Decimal,

    // Market filters
    /// ML strategy: only trade markets expiring within this many hours
    pub ml_market_expiry_window_hours: f64,
    /// MR strategy: only trade markets expiring within this many hours (shorter = more volatile)
    pub mr_market_expiry_window_hours: f64,
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
    pub test_mr_threshold_min: Decimal,
    pub test_mr_threshold_max: Decimal,
    pub test_mr_bet_usd_min: Decimal,
    pub test_mr_bet_usd_max: Decimal,
    pub test_ml_threshold_min: f64,
    pub test_ml_threshold_max: f64,

    // MR threshold auto-tuning
    /// Enable automatic MR threshold selection based on last-hour paper performance
    pub mr_autotune_mode: bool,
    pub mr_autotune_threshold_min: Decimal,
    pub mr_autotune_threshold_max: Decimal,
    /// How often to re-evaluate and update the threshold (seconds)
    pub mr_autotune_interval_sec: u64,

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

            mean_reversion_threshold: decimal_env("MEAN_REVERSION_THRESHOLD", "0.10")?,
            mean_reversion_budget_pct: decimal_env("MEAN_REVERSION_BUDGET_PCT", "0.25")?,

            ml_model_path: env_or("ML_MODEL_PATH", "research/mr_classifier_xgboost.onnx"),
            ml_win_prob_threshold: env_or("ML_WIN_PROB_THRESHOLD", "0.55").parse()?,
            ml_reversion_threshold: decimal_env("ML_REVERSION_THRESHOLD", "0.10")?,

            ml_bet_usd: decimal_env("ML_TRADE_BET_USD", "15.0")?,
            mr_bet_usd: decimal_env("MR_TRADE_BET_USD", "10.0")?,

            ml_market_expiry_window_hours: env_or("ML_MARKET_EXPIRY_WINDOW_HOURS", "48.0").parse()?,
            mr_market_expiry_window_hours: env_or("MR_MARKET_EXPIRY_WINDOW_HOURS", "0.5").parse()?,
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
            test_mr_threshold_min: decimal_env("MEAN_REVERSION_THRESHOLD_MIN", "0.10")?,
            test_mr_threshold_max: decimal_env("MEAN_REVERSION_THRESHOLD_MAX", "0.50")?,
            test_mr_bet_usd_min: decimal_env("TRADE_BET_USD_MIN", "1.0")?,
            test_mr_bet_usd_max: decimal_env("TRADE_BET_USD_MAX", "10.0")?,
            test_ml_threshold_min: env_or("ML_TEST_THRESHOLD_MIN", "0.50").parse()?,
            test_ml_threshold_max: env_or("ML_TEST_THRESHOLD_MAX", "0.80").parse()?,

            mr_autotune_mode: env_or("MR_AUTOTUNE_MODE", "false").to_lowercase() == "true",
            mr_autotune_threshold_min: decimal_env("MR_AUTOTUNE_THRESHOLD_MIN", "0.10")?,
            mr_autotune_threshold_max: decimal_env("MR_AUTOTUNE_THRESHOLD_MAX", "0.90")?,
            mr_autotune_interval_sec: env_or("MR_AUTOTUNE_INTERVAL_SEC", "3600").parse()?,

            aws_region: env_or("AWS_REGION", "eu-west-1"),
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
        if let Some(v) = s.ml_market_expiry_window_hours {
            self.ml_market_expiry_window_hours = v;
        }
        if let Some(v) = s.mean_reversion_budget_pct {
            self.mean_reversion_budget_pct = v;
        }
        if let Some(v) = s.max_open_positions {
            self.max_open_positions = v;
        }
        if let Some(v) = s.ml_reversion_threshold {
            self.ml_reversion_threshold = v;
        }
        if let Some(v) = s.ml_win_prob_threshold {
            self.ml_win_prob_threshold = v;
        }
        if let Some(v) = s.ml_bet_usd {
            self.ml_bet_usd = v;
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn decimal_env(key: &str, default: &str) -> Result<Decimal, rust_decimal::Error> {
    Decimal::from_str(&env_or(key, default))
}
