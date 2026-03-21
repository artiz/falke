#![allow(dead_code)]
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;
use chrono::Utc;
use std::collections::HashMap;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::error::{FalkeError, Result};
use crate::trading::portfolio::Portfolio;

use super::models::{GlobalSettings, TradeRecord, User};

/// DynamoDB client wrapper for Falke
pub struct DynamoStore {
    client: Client,
    users_table: String,
    trades_table: String,
    sessions_table: String,
    settings_table: String,
}

impl DynamoStore {
    pub async fn new(config: &Config) -> Result<Self> {
        let region = aws_sdk_dynamodb::config::Region::new(config.aws_region.clone());

        let sdk_config = if let Some(ref endpoint) = config.dynamo_endpoint {
            info!("Using local DynamoDB endpoint: {endpoint}");
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(region)
                .endpoint_url(endpoint)
                .test_credentials() // Use dummy credentials for LocalStack
                .load()
                .await
        } else {
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(region)
                .load()
                .await
        };

        let client = Client::new(&sdk_config);
        let prefix = format!(
            "{}-{}",
            config.dynamo_table_prefix,
            std::env::var("ENVIRONMENT").unwrap_or_else(|_| "dev".into())
        );

        let store = Self {
            client,
            users_table: format!("{prefix}-users"),
            trades_table: format!("{prefix}-trades"),
            sessions_table: format!("{prefix}-sessions"),
            settings_table: format!("{prefix}-settings"),
        };

        // Verify connectivity
        match store.client.list_tables().limit(1).send().await {
            Ok(resp) => {
                let tables = resp.table_names();
                info!("DynamoDB connected. Tables found: {}", tables.join(", "));
            }
            Err(e) => {
                warn!("DynamoDB connectivity check failed: {e}. Sessions will not be persisted.");
            }
        }

        Ok(store)
    }

    // ─── Session persistence ────────────────────────────────────────

    /// Save a user's portfolio session to DynamoDB
    pub async fn save_session(&self, portfolio: &Portfolio) -> Result<()> {
        let json = serde_json::to_string(portfolio)
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to serialize portfolio: {e}")))?;

        self.client
            .put_item()
            .table_name(&self.sessions_table)
            .item("user_id", AttributeValue::N(portfolio.user_id.to_string()))
            .item("portfolio_json", AttributeValue::S(json))
            .item("updated_at", AttributeValue::S(Utc::now().to_rfc3339()))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to save session: {e}")))?;

        Ok(())
    }

    /// Load all saved sessions from DynamoDB
    pub async fn load_all_sessions(&self) -> Result<HashMap<i64, Portfolio>> {
        let mut sessions = HashMap::new();
        let mut last_key: Option<HashMap<String, AttributeValue>> = None;

        // DynamoDB scan returns at most 1MB per page — paginate to get all sessions.
        // With 100 test portfolios each containing hundreds of trade records, the
        // total JSON easily exceeds 1MB and a single scan would silently drop items.
        loop {
            let mut req = self.client.scan().table_name(&self.sessions_table);
            if let Some(ref key) = last_key {
                req = req.set_exclusive_start_key(Some(key.clone()));
            }

            let result = req
                .send()
                .await
                .map_err(|e| FalkeError::DynamoDb(format!("Failed to scan sessions: {e}")))?;

            for item in result.items() {
                let user_id = item
                    .get("user_id")
                    .and_then(|v| v.as_n().ok())
                    .and_then(|s| s.parse::<i64>().ok());

                let json = item.get("portfolio_json").and_then(|v| v.as_s().ok());

                if let (Some(uid), Some(json_str)) = (user_id, json) {
                    match serde_json::from_str::<Portfolio>(json_str) {
                        Ok(portfolio) => {
                            info!(
                                "Restored session for user {uid}: balance=${:.2}, {} open positions, {} trades",
                                portfolio.balance,
                                portfolio.num_open_positions(),
                                portfolio.trade_history.len(),
                            );
                            sessions.insert(uid, portfolio);
                        }
                        Err(e) => {
                            error!("Failed to deserialize session for user {uid}: {e}");
                        }
                    }
                }
            }

            // If LastEvaluatedKey is set, there are more pages to fetch
            match result.last_evaluated_key {
                Some(key) if !key.is_empty() => last_key = Some(key),
                _ => break,
            }
        }

        Ok(sessions)
    }

    /// Delete a session
    pub async fn delete_session(&self, user_id: i64) -> Result<()> {
        self.client
            .delete_item()
            .table_name(&self.sessions_table)
            .key("user_id", AttributeValue::N(user_id.to_string()))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to delete session: {e}")))?;

        Ok(())
    }

    // ─── User persistence ───────────────────────────────────────────

    /// Create a new user or update existing
    pub async fn put_user(&self, user: &User) -> Result<()> {
        let mut item = HashMap::new();
        item.insert(
            "telegram_id".into(),
            AttributeValue::N(user.telegram_id.to_string()),
        );
        item.insert(
            "phone_number".into(),
            AttributeValue::S(user.phone_number.clone()),
        );
        item.insert(
            "trading_mode".into(),
            AttributeValue::S(user.trading_mode.clone()),
        );
        item.insert("is_active".into(), AttributeValue::Bool(user.is_active));
        item.insert(
            "created_at".into(),
            AttributeValue::S(user.created_at.to_rfc3339()),
        );
        item.insert(
            "last_active".into(),
            AttributeValue::S(user.last_active.to_rfc3339()),
        );

        if let Some(ref username) = user.username {
            item.insert("username".into(), AttributeValue::S(username.clone()));
        }
        if let Some(ref wallet) = user.wallet_address {
            item.insert("wallet_address".into(), AttributeValue::S(wallet.clone()));
        }

        self.client
            .put_item()
            .table_name(&self.users_table)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to put user: {e}")))?;

        info!("Saved user: telegram_id={}", user.telegram_id);
        Ok(())
    }

    /// Get a user by telegram ID
    pub async fn get_user(&self, telegram_id: i64) -> Result<Option<User>> {
        let result = self
            .client
            .get_item()
            .table_name(&self.users_table)
            .key("telegram_id", AttributeValue::N(telegram_id.to_string()))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to get user: {e}")))?;

        match result.item {
            Some(item) => Ok(Some(item_to_user(&item)?)),
            None => Ok(None),
        }
    }

    // ─── Trade persistence ──────────────────────────────────────────

    /// Save a trade record
    pub async fn put_trade(&self, trade: &TradeRecord) -> Result<()> {
        let mut item = HashMap::new();
        item.insert("trade_id".into(), AttributeValue::S(trade.trade_id.clone()));
        item.insert(
            "user_id".into(),
            AttributeValue::N(trade.user_id.to_string()),
        );
        item.insert(
            "condition_id".into(),
            AttributeValue::S(trade.condition_id.clone()),
        );
        item.insert("token_id".into(), AttributeValue::S(trade.token_id.clone()));
        item.insert("question".into(), AttributeValue::S(trade.question.clone()));
        item.insert(
            "outcome_name".into(),
            AttributeValue::S(trade.outcome_name.clone()),
        );
        item.insert("side".into(), AttributeValue::S(trade.side.clone()));
        item.insert(
            "entry_price".into(),
            AttributeValue::S(trade.entry_price.clone()),
        );
        item.insert("quantity".into(), AttributeValue::S(trade.quantity.clone()));
        item.insert(
            "cost_basis".into(),
            AttributeValue::S(trade.cost_basis.clone()),
        );
        item.insert("strategy".into(), AttributeValue::S(trade.strategy.clone()));
        item.insert("mode".into(), AttributeValue::S(trade.mode.clone()));
        item.insert("status".into(), AttributeValue::S(trade.status.clone()));
        item.insert(
            "opened_at".into(),
            AttributeValue::S(trade.opened_at.to_rfc3339()),
        );

        if let Some(ref exit_price) = trade.exit_price {
            item.insert("exit_price".into(), AttributeValue::S(exit_price.clone()));
        }
        if let Some(ref pnl) = trade.realized_pnl {
            item.insert("realized_pnl".into(), AttributeValue::S(pnl.clone()));
        }
        if let Some(ref closed_at) = trade.closed_at {
            item.insert(
                "closed_at".into(),
                AttributeValue::S(closed_at.to_rfc3339()),
            );
        }

        self.client
            .put_item()
            .table_name(&self.trades_table)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to put trade: {e}")))?;

        Ok(())
    }

    // ─── Global settings persistence ────────────────────────────────

    /// Load persisted global settings. Returns defaults if the table/item doesn't exist.
    pub async fn load_global_settings(&self) -> Result<GlobalSettings> {
        let result = self
            .client
            .get_item()
            .table_name(&self.settings_table)
            .key("settings_id", AttributeValue::S("global".into()))
            .send()
            .await;

        let item = match result {
            Ok(r) => match r.item {
                Some(i) => i,
                None => return Ok(GlobalSettings::default()),
            },
            Err(e) => {
                // Table may not exist yet — treat as empty settings, not a hard error
                warn!("Could not read settings table: {e}. Using env defaults.");
                return Ok(GlobalSettings::default());
            }
        };

        Ok(GlobalSettings {
            paused: item
                .get("paused")
                .and_then(|v| v.as_bool().ok())
                .copied()
                .unwrap_or(false),
            trading_mode: get_s_opt(&item, "trading_mode"),
            ml_market_expiry_window_hours: get_s_opt(&item, "ml_market_expiry_window_hours")
                .and_then(|s| s.parse().ok()),
            max_open_positions: get_s_opt(&item, "max_open_positions")
                .and_then(|s| s.parse().ok()),
        })
    }

    /// Persist global settings (upsert).
    pub async fn save_global_settings(&self, s: &GlobalSettings) -> Result<()> {
        let mut item = HashMap::new();
        item.insert("settings_id".into(), AttributeValue::S("global".into()));
        item.insert("paused".into(), AttributeValue::Bool(s.paused));
        item.insert(
            "updated_at".into(),
            AttributeValue::S(Utc::now().to_rfc3339()),
        );
        if let Some(ref mode) = s.trading_mode {
            item.insert("trading_mode".into(), AttributeValue::S(mode.clone()));
        }

        if let Some(v) = s.ml_market_expiry_window_hours {
            item.insert(
                "ml_market_expiry_window_hours".into(),
                AttributeValue::S(v.to_string()),
            );
        }
        if let Some(v) = s.max_open_positions {
            item.insert(
                "max_open_positions".into(),
                AttributeValue::S(v.to_string()),
            );
        }

        self.client
            .put_item()
            .table_name(&self.settings_table)
            .set_item(Some(item))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to save settings: {e}")))?;

        Ok(())
    }

    /// Delete all sessions (used when trading mode changes)
    pub async fn clear_all_sessions(&self) -> Result<()> {
        let result = self
            .client
            .scan()
            .table_name(&self.sessions_table)
            .projection_expression("user_id")
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to scan sessions for clear: {e}")))?;

        for item in result.items() {
            if let Some(uid) = item.get("user_id").and_then(|v| v.as_n().ok()) {
                self.client
                    .delete_item()
                    .table_name(&self.sessions_table)
                    .key("user_id", AttributeValue::N(uid.clone()))
                    .send()
                    .await
                    .map_err(|e| {
                        FalkeError::DynamoDb(format!("Failed to delete session {uid}: {e}"))
                    })?;
            }
        }

        Ok(())
    }

    /// Get all trades for a user
    pub async fn get_user_trades(&self, user_id: i64) -> Result<Vec<TradeRecord>> {
        let result = self
            .client
            .query()
            .table_name(&self.trades_table)
            .index_name("user_id-index")
            .key_condition_expression("user_id = :uid")
            .expression_attribute_values(":uid", AttributeValue::N(user_id.to_string()))
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to query trades: {e}")))?;

        let trades = result
            .items()
            .iter()
            .filter_map(|item| item_to_trade(item).ok())
            .collect();

        Ok(trades)
    }
}

fn get_s(item: &HashMap<String, AttributeValue>, key: &str) -> Result<String> {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| FalkeError::DynamoDb(format!("Missing field: {key}")))
}

fn get_n(item: &HashMap<String, AttributeValue>, key: &str) -> Result<i64> {
    item.get(key)
        .and_then(|v| v.as_n().ok())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| FalkeError::DynamoDb(format!("Missing numeric field: {key}")))
}

fn get_s_opt(item: &HashMap<String, AttributeValue>, key: &str) -> Option<String> {
    item.get(key)
        .and_then(|v| v.as_s().ok())
        .map(|s| s.to_string())
}

fn item_to_user(item: &HashMap<String, AttributeValue>) -> Result<User> {
    Ok(User {
        telegram_id: get_n(item, "telegram_id")?,
        username: get_s_opt(item, "username"),
        phone_number: get_s(item, "phone_number")?,
        wallet_address: get_s_opt(item, "wallet_address"),
        trading_mode: get_s(item, "trading_mode").unwrap_or_else(|_| "paper".into()),
        is_active: item
            .get("is_active")
            .and_then(|v| v.as_bool().ok())
            .copied()
            .unwrap_or(false),
        created_at: get_s(item, "created_at")
            .ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
        last_active: get_s(item, "last_active")
            .ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
    })
}

fn item_to_trade(item: &HashMap<String, AttributeValue>) -> Result<TradeRecord> {
    Ok(TradeRecord {
        trade_id: get_s(item, "trade_id")?,
        user_id: get_n(item, "user_id")?,
        condition_id: get_s(item, "condition_id")?,
        token_id: get_s(item, "token_id")?,
        question: get_s(item, "question")?,
        outcome_name: get_s(item, "outcome_name")?,
        side: get_s(item, "side")?,
        entry_price: get_s(item, "entry_price")?,
        exit_price: get_s_opt(item, "exit_price"),
        quantity: get_s(item, "quantity")?,
        cost_basis: get_s(item, "cost_basis")?,
        realized_pnl: get_s_opt(item, "realized_pnl"),
        strategy: get_s(item, "strategy")?,
        mode: get_s(item, "mode")?,
        status: get_s(item, "status")?,
        opened_at: get_s(item, "opened_at")
            .ok()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(Utc::now),
        closed_at: get_s_opt(item, "closed_at")
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc)),
    })
}
