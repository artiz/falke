variable "aws_region" {
  description = "AWS region for deployment"
  type        = string
  default     = "eu-west-2" # London — closest to Polymarket servers
}

variable "project_name" {
  description = "Project name for resource naming"
  type        = string
  default     = "falke"
}

variable "environment" {
  description = "Environment (dev, prod)"
  type        = string
  default     = "dev"
}

variable "ecs_cpu" {
  description = "ECS task CPU units (256 = 0.25 vCPU)"
  type        = number
  default     = 256
}

variable "ecs_memory" {
  description = "ECS task memory in MiB"
  type        = number
  default     = 512
}

# === Secrets (managed via AWS Secrets Manager) ===

variable "telegram_bot_token" {
  description = "Telegram bot token from BotFather"
  type        = string
  sensitive   = true
}

variable "allowed_phones" {
  description = "Comma-separated list of allowed phone numbers"
  type        = string
  sensitive   = true
}



# === Trading configuration ===

variable "trading_mode" {
  description = "Trading mode: paper or live"
  type        = string
  default     = "paper"
}

variable "paper_balance" {
  description = "Initial paper trading balance in USD"
  type        = string
  default     = "500.0"
}

# === ML strategy parameters ===

variable "ml_model_path" {
  description = "Path to the ONNX model file inside the container (empty = ML disabled)"
  type        = string
  default     = "research/mr_classifier_xgboost.onnx"
}

variable "ml_win_prob_threshold" {
  description = "Minimum win probability from the ML model to take a trade (e.g. 0.55 = 55%)"
  type        = string
  default     = "0.55"
}

# === Risk / engine parameters ===

variable "trade_poll_interval_sec" {
  description = "How often the trading engine polls for signals (seconds)"
  type        = string
  default     = "1"
}

variable "max_bet_usd" {
  description = "Maximum single bet size in USD"
  type        = string
  default     = "50.0"
}

variable "max_open_positions" {
  description = "Maximum number of open positions per user"
  type        = string
  default     = "300"
}

variable "cooldown_sec" {
  description = "Cooldown period between trades on the same outcome (seconds)"
  type        = string
  default     = "600"
}

variable "pnl_notify_threshold_usd" {
  description = "Send P&L notification when P&L crosses this USD threshold"
  type        = string
  default     = "50.0"
}

variable "budget_brake_pct" {
  description = "Circuit breaker: pause trading if portfolio loss exceeds this % of initial balance (0 = disabled)"
  type        = string
  default     = "20.0"
}

variable "budget_brake_time_sec" {
  description = "How long to pause trading (seconds) when the budget brake fires"
  type        = string
  default     = "7200"
}

variable "ml_market_expiry_window_hours" {
  description = "ML strategy: only trade markets expiring within this many hours"
  type        = string
  default     = "48.0"
}

variable "mr_market_expiry_window_hours" {
  description = "MR strategy: only trade markets expiring within this many hours (0.5 = 30 min)"
  type        = string
  default     = "0.5"
}

variable "ignored_topics" {
  description = "Comma-separated list of Polymarket topic slugs to ignore (e.g. politics)"
  type        = string
  default     = "politics"
}

variable "polygon_rpc_url" {
  description = "Polygon RPC URL for live trading wallet interactions"
  type        = string
  default     = "https://polygon-bor-rpc.publicnode.com"
}

variable "process_usdc_allowances" {
  description = "Set to true once to approve USDC allowance for a new wallet (then set back to false)"
  type        = string
  default     = "false"
}

variable "min_liquidity_usd" {
  description = "Minimum market liquidity to consider trading"
  type        = string
  default     = "1000.0"
}

# === Testing / parameter sweep ===

variable "testing_mode" {
  description = "Enable parameter sweep testing mode (runs multiple portfolios with different params)"
  type        = string
  default     = "false"
}

# === Mean Reversion strategy ===

variable "mean_reversion_threshold" {
  description = "Min price % change to trigger a mean reversion signal (e.g. 0.20 = 20%)"
  type        = string
  default     = "0.10"
}

variable "mean_reversion_budget_pct" {
  description = "Fraction of trades allocated to plain MR (0.0 = ML only, 1.0 = MR only)"
  type        = string
  default     = "0.25"
}

variable "trade_bet_usd" {
  description = "Fixed bet size per MR/ML position in USD (shared)"
  type        = string
  default     = "5.0"
}

variable "ml_reversion_threshold" {
  description = "Minimum price % change pre-filter for ML scan (e.g. 0.10 = 10%)"
  type        = string
  default     = "0.10"
}

variable "mean_reversion_threshold_min" {
  description = "Minimum MR threshold for testing sweep"
  type        = string
  default     = "0.10"
}

variable "mean_reversion_threshold_max" {
  description = "Maximum MR threshold for testing sweep"
  type        = string
  default     = "0.90"
}

variable "trade_bet_usd_min" {
  description = "Minimum bet size for testing sweep in USD"
  type        = string
  default     = "1.0"
}

variable "trade_bet_usd_max" {
  description = "Maximum bet size for testing sweep in USD"
  type        = string
  default     = "10.0"
}

variable "ml_test_threshold_min" {
  description = "Minimum ML win-prob threshold for testing sweep"
  type        = string
  default     = "0.50"
}

variable "ml_test_threshold_max" {
  description = "Maximum ML win-prob threshold for testing sweep"
  type        = string
  default     = "0.80"
}
