use ethers::signers::{LocalWallet, Signer};
use std::str::FromStr;

use crate::error::{FalkeError, Result};

/// Parse and validate an Ethereum private key
pub fn parse_wallet(private_key: &str) -> Result<LocalWallet> {
    // Strip 0x prefix if present
    let key = private_key.strip_prefix("0x").unwrap_or(private_key);

    LocalWallet::from_str(key)
        .map_err(|e| FalkeError::Wallet(format!("Invalid private key: {e}")))
}

/// Get the public address from a private key
pub fn wallet_address(private_key: &str) -> Result<String> {
    let wallet = parse_wallet(private_key)?;
    Ok(format!("{:?}", wallet.address()))
}
