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

# === Tail Risk strategy parameters ===

variable "tail_risk_max_price" {
  description = "Maximum outcome price to consider (e.g. 0.01 = 1 cent)"
  type        = string
  default     = "0.01"
}

variable "tail_risk_bet_usd" {
  description = "Minimum bet size per position in USD"
  type        = string
  default     = "10.0"
}

variable "tail_risk_kelly_edge_multiplier" {
  description = "Edge multiplier for Kelly criterion (true prob = market price × multiplier)"
  type        = string
  default     = "2.0"
}

variable "tail_risk_min_payout_multiplier" {
  description = "Minimum payout multiplier to enter a position (e.g. 25 = only outcomes paying 25x+)"
  type        = string
  default     = "25.0"
}

variable "tail_risk_take_profit_fraction" {
  description = "Fraction of positions assigned a take-profit exit (0.0 = disabled)"
  type        = string
  default     = "0.0"
}

variable "tail_risk_take_profit_pct" {
  description = "Take-profit threshold as percentage gain (0.0 = disabled)"
  type        = string
  default     = "0.0"
}

variable "tail_risk_stop_loss_pct" {
  description = "Stop-loss threshold as percentage loss (0.0 = disabled)"
  type        = string
  default     = "0.0"
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

variable "market_expiry_window_hours" {
  description = "Only track markets expiring within this many hours"
  type        = string
  default     = "6"
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

variable "tail_risk_max_price_min" {
  description = "Minimum max-price for testing sweep (e.g. 0.01)"
  type        = string
  default     = "0.01"
}

variable "tail_risk_max_price_max" {
  description = "Maximum max-price for testing sweep (e.g. 0.10)"
  type        = string
  default     = "0.10"
}

variable "tail_risk_bet_usd_min" {
  description = "Minimum bet size for testing sweep in USD"
  type        = string
  default     = "1.0"
}

variable "tail_risk_bet_usd_max" {
  description = "Maximum bet size for testing sweep in USD"
  type        = string
  default     = "10.0"
}

# === Mean Reversion strategy ===

variable "mean_reversion_threshold" {
  description = "Min price % change to trigger a mean reversion signal (e.g. 0.20 = 20%)"
  type        = string
  default     = "0.10"
}

variable "mean_reversion_budget_pct" {
  description = "Fraction of trades allocated to MR (0.0 = disabled, 1.0 = 100% MR)"
  type        = string
  default     = "100.0"
}

variable "mean_reversion_bet_usd" {
  description = "Fixed bet size per MR position in USD"
  type        = string
  default     = "5.0"
}

variable "mean_reversion_threshold_min" {
  description = "Minimum MR threshold for testing sweep"
  type        = string
  default     = "0.10"
}

variable "mean_reversion_threshold_max" {
  description = "Maximum MR threshold for testing sweep"
  type        = string
  default     = "0.50"
}
