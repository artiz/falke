use std::str::FromStr;
use tracing::info;

use alloy::signers::Signer as _;  // needed for .with_chain_id()
use alloy::signers::local::PrivateKeySigner;
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::Normal;
use polymarket_client_sdk::clob::{Client as SdkClient, Config as SdkConfig};
use polymarket_client_sdk::POLYGON;

pub type AuthenticatedClient = SdkClient<Authenticated<Normal>>;

use crate::config::Config;
use crate::error::{FalkeError, Result};

/// Authenticate with Polymarket CLOB using a wallet private key.
/// Creates new API credentials or derives existing ones — all in one step.
pub async fn authenticate_live(config: &Config) -> Result<(AuthenticatedClient, PrivateKeySigner)> {
    let private_key = config
        .wallet_private_key
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FalkeError::Wallet("WALLET_PRIVATE_KEY is required for live trading".into()))?;

    let signer = PrivateKeySigner::from_str(private_key)
        .map_err(|e| FalkeError::Wallet(format!("Invalid private key: {e}")))?
        .with_chain_id(Some(POLYGON));

    let client = SdkClient::new(&config.clob_api_url, SdkConfig::default())
        .map_err(|e| FalkeError::Wallet(format!("Failed to create CLOB client: {e}")))?
        .authentication_builder(&signer)
        .authenticate()
        .await
        .map_err(|e| FalkeError::Wallet(format!("CLOB authentication failed: {e}")))?;

    info!("CLOB authenticated — wallet: {}", signer.address());

    Ok((client, signer))
}
