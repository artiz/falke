# Telegram bot token
resource "aws_secretsmanager_secret" "telegram_token" {
  name = "${local.name_prefix}/telegram-bot-token"
  tags = { Name = "${local.name_prefix}-telegram-token" }
}

resource "aws_secretsmanager_secret_version" "telegram_token" {
  secret_id     = aws_secretsmanager_secret.telegram_token.id
  secret_string = var.telegram_bot_token
}

# Allowed phone numbers
resource "aws_secretsmanager_secret" "allowed_phones" {
  name = "${local.name_prefix}/allowed-phones"
  tags = { Name = "${local.name_prefix}-allowed-phones" }
}

resource "aws_secretsmanager_secret_version" "allowed_phones" {
  secret_id     = aws_secretsmanager_secret.allowed_phones.id
  secret_string = var.allowed_phones
}

# Wallet private key (for live trading)
resource "aws_secretsmanager_secret" "wallet_key" {
  name = "${local.name_prefix}/wallet-private-key"
  tags = { Name = "${local.name_prefix}-wallet-key" }
}

resource "aws_secretsmanager_secret_version" "wallet_key" {
  secret_id     = aws_secretsmanager_secret.wallet_key.id
  secret_string = var.wallet_private_key
}

# Polymarket Relayer API key (for live trading)
resource "aws_secretsmanager_secret" "relayer_api_key" {
  name = "${local.name_prefix}/relayer-api-key"
  tags = { Name = "${local.name_prefix}-relayer-api-key" }
}

resource "aws_secretsmanager_secret_version" "relayer_api_key" {
  secret_id     = aws_secretsmanager_secret.relayer_api_key.id
  secret_string = var.relayer_api_key
}

# Polymarket Relayer API key address (for live trading)
resource "aws_secretsmanager_secret" "relayer_api_key_address" {
  name = "${local.name_prefix}/relayer-api-key-address"
  tags = { Name = "${local.name_prefix}-relayer-api-key-address" }
}

resource "aws_secretsmanager_secret_version" "relayer_api_key_address" {
  secret_id     = aws_secretsmanager_secret.relayer_api_key_address.id
  secret_string = var.relayer_api_key_address
}
