use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;
use chrono::Utc;
use std::collections::HashMap;
use tracing::info;

use crate::error::{FalkeError, Result};

use super::models::{TradeRecord, User};

/// DynamoDB client wrapper for Falke
pub struct DynamoStore {
    client: Client,
    users_table: String,
    trades_table: String,
}

impl DynamoStore {
    pub async fn new(region: &str, table_prefix: &str) -> Result<Self> {
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_sdk_dynamodb::config::Region::new(region.to_string()))
            .load()
            .await;

        let client = Client::new(&config);

        Ok(Self {
            client,
            users_table: format!("{table_prefix}-users"),
            trades_table: format!("{table_prefix}-trades"),
        })
    }

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
        item.insert(
            "is_active".into(),
            AttributeValue::Bool(user.is_active),
        );
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

    /// Save a trade record
    pub async fn put_trade(&self, trade: &TradeRecord) -> Result<()> {
        let mut item = HashMap::new();
        item.insert(
            "trade_id".into(),
            AttributeValue::S(trade.trade_id.clone()),
        );
        item.insert(
            "user_id".into(),
            AttributeValue::N(trade.user_id.to_string()),
        );
        item.insert(
            "condition_id".into(),
            AttributeValue::S(trade.condition_id.clone()),
        );
        item.insert(
            "token_id".into(),
            AttributeValue::S(trade.token_id.clone()),
        );
        item.insert(
            "question".into(),
            AttributeValue::S(trade.question.clone()),
        );
        item.insert(
            "outcome_name".into(),
            AttributeValue::S(trade.outcome_name.clone()),
        );
        item.insert("side".into(), AttributeValue::S(trade.side.clone()));
        item.insert(
            "entry_price".into(),
            AttributeValue::S(trade.entry_price.clone()),
        );
        item.insert(
            "quantity".into(),
            AttributeValue::S(trade.quantity.clone()),
        );
        item.insert(
            "cost_basis".into(),
            AttributeValue::S(trade.cost_basis.clone()),
        );
        item.insert(
            "strategy".into(),
            AttributeValue::S(trade.strategy.clone()),
        );
        item.insert("mode".into(), AttributeValue::S(trade.mode.clone()));
        item.insert("status".into(), AttributeValue::S(trade.status.clone()));
        item.insert(
            "opened_at".into(),
            AttributeValue::S(trade.opened_at.to_rfc3339()),
        );

        if let Some(ref exit_price) = trade.exit_price {
            item.insert(
                "exit_price".into(),
                AttributeValue::S(exit_price.clone()),
            );
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

    /// Get all trades for a user
    pub async fn get_user_trades(&self, user_id: i64) -> Result<Vec<TradeRecord>> {
        let result = self
            .client
            .query()
            .table_name(&self.trades_table)
            .index_name("user_id-index")
            .key_condition_expression("user_id = :uid")
            .expression_attribute_values(
                ":uid",
                AttributeValue::N(user_id.to_string()),
            )
            .send()
            .await
            .map_err(|e| FalkeError::DynamoDb(format!("Failed to query trades: {e}")))?;

        let trades = result
            .items
            .unwrap_or_default()
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
    item.get(key).and_then(|v| v.as_s().ok()).map(|s| s.to_string())
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
