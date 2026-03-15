use crate::config::Config;
use crate::error::{FalkeError, Result};
use tracing::info;

/// CLOB API credentials derived from wallet signing
#[derive(Debug, Clone)]
pub struct ClobCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
}

/// Relayer API credentials (two-field, from Polymarket UI → Profile → API Keys)
#[derive(Debug, Clone)]
pub struct RelayerCredentials {
    pub api_key: String,
    pub api_key_address: String,
}

/// Resolve Relayer credentials from config.
/// Returns Some if RELAYER_API_KEY + RELAYER_API_KEY_ADDRESS are both set.
pub fn resolve_relayer_credentials(config: &Config) -> Option<RelayerCredentials> {
    match (&config.relayer_api_key, &config.relayer_api_key_address) {
        (Some(key), Some(address)) => {
            info!("Using Relayer API credentials");
            Some(RelayerCredentials {
                api_key: key.clone(),
                api_key_address: address.clone(),
            })
        }
        _ => None,
    }
}

/// Derive CLOB API credentials from a wallet private key.
///
/// Polymarket's CLOB requires you to sign a message with your Ethereum wallet
/// to derive API credentials. This uses EIP-712 typed data signing.
///
/// See: https://docs.polymarket.com/#get-api-key
pub async fn derive_clob_credentials(private_key: &str) -> Result<ClobCredentials> {
    use ethers::signers::{LocalWallet, Signer};
    use std::str::FromStr;

    let wallet = LocalWallet::from_str(private_key)
        .map_err(|e| FalkeError::Wallet(format!("Invalid private key: {e}")))?;

    let address = wallet.address();
    info!("Deriving CLOB credentials for wallet: {address:?}");

    // Step 1: Request a nonce from the CLOB API
    let client = reqwest::Client::new();
    let nonce_resp = client
        .get("https://clob.polymarket.com/auth/nonce")
        .query(&[("address", format!("{address:?}"))])
        .send()
        .await
        .map_err(|e| FalkeError::PolymarketApi(format!("Failed to get nonce: {e}")))?;

    if !nonce_resp.status().is_success() {
        let body = nonce_resp.text().await.unwrap_or_default();
        return Err(FalkeError::PolymarketApi(format!(
            "Nonce request failed: {body}"
        )));
    }

    let nonce: String = nonce_resp
        .json()
        .await
        .map_err(|e| FalkeError::PolymarketApi(format!("Failed to parse nonce: {e}")))?;

    // Step 2: Sign the nonce with the wallet
    let signature = wallet
        .sign_message(nonce.as_bytes())
        .await
        .map_err(|e| FalkeError::Wallet(format!("Failed to sign nonce: {e}")))?;

    let sig_hex = format!("0x{}", hex::encode(signature.to_vec()));

    // Step 3: Exchange signature for API credentials
    let derive_resp = client
        .post("https://clob.polymarket.com/auth/derive-api-key")
        .json(&serde_json::json!({
            "address": format!("{address:?}"),
            "signature": sig_hex,
            "nonce": nonce,
        }))
        .send()
        .await
        .map_err(|e| FalkeError::PolymarketApi(format!("Failed to derive API key: {e}")))?;

    if !derive_resp.status().is_success() {
        let body = derive_resp.text().await.unwrap_or_default();
        return Err(FalkeError::PolymarketApi(format!(
            "API key derivation failed: {body}"
        )));
    }

    #[derive(serde::Deserialize)]
    struct DeriveResponse {
        #[serde(rename = "apiKey")]
        api_key: String,
        secret: String,
        passphrase: String,
    }

    let creds: DeriveResponse = derive_resp
        .json()
        .await
        .map_err(|e| FalkeError::PolymarketApi(format!("Failed to parse API credentials: {e}")))?;

    info!("Successfully derived CLOB API credentials");

    Ok(ClobCredentials {
        api_key: creds.api_key,
        api_secret: creds.secret,
        api_passphrase: creds.passphrase,
    })
}
