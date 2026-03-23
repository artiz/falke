# Users table
resource "aws_dynamodb_table" "users" {
  name         = "${local.name_prefix}-users"
  billing_mode = "PAY_PER_REQUEST" # On-demand pricing — no capacity planning needed
  hash_key     = "telegram_id"

  attribute {
    name = "telegram_id"
    type = "N"
  }

  tags = { Name = "${local.name_prefix}-users" }
}

# Trades table
resource "aws_dynamodb_table" "trades" {
  name         = "${local.name_prefix}-trades"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "trade_id"

  attribute {
    name = "trade_id"
    type = "S"
  }

  attribute {
    name = "user_id"
    type = "N"
  }

  # GSI for querying trades by user
  global_secondary_index {
    name            = "user_id-index"
    hash_key        = "user_id"
    projection_type = "ALL"
  }

  tags = { Name = "${local.name_prefix}-trades" }
}

# Sessions table — stores serialized portfolios for session restore
resource "aws_dynamodb_table" "sessions" {
  name         = "${local.name_prefix}-sessions"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "user_id"

  attribute {
    name = "user_id"
    type = "N"
  }

  # Stream needed for portfolio metrics Lambda
  stream_enabled   = true
  stream_view_type = "NEW_IMAGE"

  tags = { Name = "${local.name_prefix}-sessions" }
}

# Settings table — stores global bot settings (paused state, strategy params)
resource "aws_dynamodb_table" "settings" {
  name         = "${local.name_prefix}-settings"
  billing_mode = "PAY_PER_REQUEST"
  hash_key     = "settings_id"

  attribute {
    name = "settings_id"
    type = "S"
  }

  tags = { Name = "${local.name_prefix}-settings" }
}
