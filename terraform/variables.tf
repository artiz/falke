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
  description = "Environment (dev, staging, prod)"
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

variable "wallet_private_key" {
  description = "Ethereum/Polygon wallet private key for live trading"
  type        = string
  sensitive   = true
  default     = ""
}

variable "trading_mode" {
  description = "Trading mode: paper or live"
  type        = string
  default     = "paper"
}

variable "paper_balance" {
  description = "Initial paper trading balance in USD"
  type        = string
  default     = "1000.0"
}
