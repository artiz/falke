use std::str::FromStr;

use alloy::signers::local::PrivateKeySigner;
use polymarket_client_sdk::clob::types::request::UpdateBalanceAllowanceRequest;
use polymarket_client_sdk::clob::types::Side;
use polymarket_client_sdk::data::types::request::PositionsRequest;
use polymarket_client_sdk::data::Client as DataClient;
use polymarket_client_sdk::types::U256;
use rust_decimal::Decimal;
use tracing::debug;

use super::auth::AuthenticatedClient;
use crate::error::{FalkeError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Simplified position for reconciliation (mirrors SDK's `data::Position`).
#[derive(Debug, Clone)]
pub struct ClobPosition {
    pub asset_id: String, // U256 as decimal string
    pub size: Decimal,
    pub avg_price: Decimal,
    pub cur_price: Decimal,
    pub condition_id: String,
}

impl ClobPosition {
    pub fn size_f64(&self) -> f64 {
        self.size.to_string().parse().unwrap_or(0.0)
    }
}

/// Thin wrapper around the official Polymarket SDK clients.
pub struct ClobClient {
    clob: AuthenticatedClient,
    signer: PrivateKeySigner,
    data_api_url: String,
    polygon_rpc_url: String,
    process_usdc_allowances: bool,
}

impl ClobClient {
    pub fn new(
        clob: AuthenticatedClient,
        signer: PrivateKeySigner,
        polygon_rpc_url: String,
        process_usdc_allowances: bool,
    ) -> Self {
        Self {
            clob,
            signer,
            data_api_url: "https://data-api.polymarket.com".to_string(),
            polygon_rpc_url,
            process_usdc_allowances,
        }
    }

    /// Place a limit order for a given token.
    pub async fn place_order(
        &self,
        token_id: &str,
        side: OrderSide,
        price: Decimal,
        size: Decimal,
    ) -> Result<String> {
        let token_u256 = U256::from_str(token_id).map_err(|e| {
            FalkeError::OrderRejected(format!("Invalid token_id '{token_id}': {e}"))
        })?;

        // Polymarket minimum tick size is 0.001 — round price to 3 decimal places
        let price = price.round_dp(3);
        // Size must also be a whole number of cents (2 dp)
        let size = size.round_dp(2);

        let sdk_side = match side {
            OrderSide::Buy => Side::Buy,
            OrderSide::Sell => Side::Sell,
        };

        let unsigned = self
            .clob
            .limit_order()
            .token_id(token_u256)
            .price(price)
            .size(size)
            .side(sdk_side)
            .build()
            .await
            .map_err(|e| FalkeError::OrderRejected(format!("Failed to build order: {e}")))?;

        let signed = self
            .clob
            .sign(&self.signer, unsigned)
            .await
            .map_err(|e| FalkeError::OrderRejected(format!("Failed to sign order: {e}")))?;

        let resp = self
            .clob
            .post_order(signed)
            .await
            .map_err(|e| FalkeError::OrderRejected(format!("{e}")))?;

        debug!("Order placed: {} success={}", resp.order_id, resp.success);
        Ok(resp.order_id)
    }

    /// Set on-chain USDC allowances for Polymarket exchange contracts, then refresh CLOB cache.
    pub async fn ensure_allowance(&self) -> Result<()> {
        use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
        use tracing::info;

        let address = self.signer.address();
        info!("Live wallet address: {address}  (verify this matches your Polymarket account)");

        // Step 1: Optionally set on-chain ERC-20 approvals (enabled via PROCESS_USDC_ALLOWANCES=true)
        if self.process_usdc_allowances {
            super::on_chain::ensure_usdc_allowances(&self.signer, &self.polygon_rpc_url).await?;
        }

        // Step 1b: Always ensure ERC-1155 approvals — required for sell orders
        // (CTF Exchange must be able to transfer your conditional tokens when a sell matches)
        if let Err(e) =
            super::on_chain::ensure_ctf_approvals(&self.signer, &self.polygon_rpc_url).await
        {
            tracing::warn!("Could not set CTF approvals: {e}. Sell orders may fail.");
        }

        // Step 2: Refresh CLOB's cached view of on-chain balance/allowances
        self.clob
            .update_balance_allowance(UpdateBalanceAllowanceRequest::default())
            .await
            .map_err(|e| {
                FalkeError::Wallet(format!("Failed to refresh CLOB allowance cache: {e}"))
            })?;

        // Step 3: Log what the CLOB now sees
        match self
            .clob
            .balance_allowance(BalanceAllowanceRequest::default())
            .await
        {
            Ok(bal) => info!(
                "CLOB balance: {} USDC | allowances: {:?}",
                bal.balance, bal.allowances
            ),
            Err(e) => tracing::warn!("Could not fetch CLOB balance: {e}"),
        }

        Ok(())
    }

    /// Fetch the current CLOB USDC balance for this wallet.
    /// The SDK returns raw USDC units (6 decimal places); divide by 1e6 for human-readable dollars.
    pub async fn balance_usdc(&self) -> Option<Decimal> {
        use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
        match self
            .clob
            .balance_allowance(BalanceAllowanceRequest::default())
            .await
        {
            Ok(bal) => Some(bal.balance / Decimal::from(1_000_000)),
            Err(_) => None,
        }
    }

    /// Fetch open positions for this wallet from the Polymarket Data API.
    pub async fn get_positions(&self) -> Result<Vec<ClobPosition>> {
        let address = self.signer.address();

        let data_client = DataClient::new(&self.data_api_url)
            .map_err(|e| FalkeError::PolymarketApi(format!("Failed to create data client: {e}")))?;

        let request = PositionsRequest::builder().user(address).build();

        let sdk_positions = data_client
            .positions(&request)
            .await
            .map_err(|e| FalkeError::PolymarketApi(format!("positions request failed: {e}")))?;

        debug!("Fetched {} position(s) for {address}", sdk_positions.len());

        let positions = sdk_positions
            .into_iter()
            .map(|p| ClobPosition {
                asset_id: p.asset.to_string(),
                size: p.size,
                avg_price: p.avg_price,
                cur_price: p.cur_price,
                condition_id: format!("{:?}", p.condition_id),
            })
            .collect();

        Ok(positions)
    }
}
