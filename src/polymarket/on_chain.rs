use std::str::FromStr;

use alloy::network::EthereumWallet;
use alloy::primitives::{Address, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use tracing::{info, warn};

use crate::error::{FalkeError, Result};

// Native USDC on Polygon (Polymarket migrated from USDC.e to native USDC)
const USDC_POLYGON: &str = "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359";

// Conditional Tokens Framework (ERC-1155) — holds the outcome tokens
const CTF_CONTRACT: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";

// Polymarket exchange contracts that need USDC spending approval (ERC-20)
// and conditional token transfer approval (ERC-1155 setApprovalForAll)
const EXCHANGE_CONTRACTS: &[&str] = &[
    "0x4bfb41d5b3570defd03c39a9a4d8de6bd8b8982e", // CTF Exchange
    "0xc5d563a36ae78145c45a50134d48a1215220f80a", // Neg Risk CTF Exchange
    "0xd91e80cf2e7be2e162c6513ced06f1dd0da35296", // Neg Risk Adapter
];

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IERC20 {
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }

    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IERC1155 {
        function isApprovedForAll(address owner, address operator) external view returns (bool);
        function setApprovalForAll(address operator, bool approved) external;
    }
}

/// Ensure the CTF contract has `setApprovalForAll` granted to each exchange contract.
/// This is required for sell orders — the exchange must be able to transfer your
/// conditional tokens (ERC-1155) when a sell is matched.
pub async fn ensure_ctf_approvals(signer: &PrivateKeySigner, rpc_url: &str) -> Result<()> {
    let wallet = EthereumWallet::from(signer.clone());
    let url: alloy::transports::http::reqwest::Url = rpc_url
        .parse()
        .map_err(|e| FalkeError::Wallet(format!("Invalid Polygon RPC URL '{rpc_url}': {e}")))?;
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

    let ctf_addr = Address::from_str(CTF_CONTRACT)
        .map_err(|e| FalkeError::Wallet(format!("Invalid CTF address: {e}")))?;
    let ctf = IERC1155::new(ctf_addr, provider.clone());

    for &contract_str in EXCHANGE_CONTRACTS {
        let operator = Address::from_str(contract_str)
            .map_err(|e| FalkeError::Wallet(format!("Invalid exchange address: {e}")))?;

        let approved = ctf
            .isApprovedForAll(signer.address(), operator)
            .call()
            .await
            .map_err(|e| {
                FalkeError::Wallet(format!("isApprovedForAll() failed for {contract_str}: {e}"))
            })?;

        if !approved {
            info!("Setting CTF setApprovalForAll for {contract_str}...");
            let pending = ctf
                .setApprovalForAll(operator, true)
                .send()
                .await
                .map_err(|e| {
                    FalkeError::Wallet(format!(
                        "setApprovalForAll() send failed for {contract_str}: {e}"
                    ))
                })?;
            pending.get_receipt().await.map_err(|e| {
                FalkeError::Wallet(format!(
                    "setApprovalForAll() receipt failed for {contract_str}: {e}"
                ))
            })?;
            info!("CTF approval granted to {contract_str}");
        } else {
            info!("CTF already approved for {contract_str}");
        }
    }

    Ok(())
}

/// Ensure EOA has approved all Polymarket exchange contracts to spend USDC.
/// Sends on-chain `approve(max)` transactions for any contract with zero allowance.
pub async fn ensure_usdc_allowances(signer: &PrivateKeySigner, rpc_url: &str) -> Result<()> {
    let wallet = EthereumWallet::from(signer.clone());
    let url: alloy::transports::http::reqwest::Url = rpc_url
        .parse()
        .map_err(|e| FalkeError::Wallet(format!("Invalid Polygon RPC URL '{rpc_url}': {e}")))?;
    let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

    let usdc_addr = Address::from_str(USDC_POLYGON)
        .map_err(|e| FalkeError::Wallet(format!("Invalid USDC address: {e}")))?;

    let usdc = IERC20::new(usdc_addr, provider.clone());

    // Log on-chain USDC balance so user can verify funds are in the right place
    match usdc.balanceOf(signer.address()).call().await {
        Ok(bal) => {
            let human = u128::try_from(bal).unwrap_or(u128::MAX) as f64 / 1_000_000.0;
            info!(
                "On-chain USDC balance for {}: ${:.2}",
                signer.address(),
                human
            );
        }
        Err(e) => warn!("Could not fetch on-chain USDC balance: {e}"),
    }

    for &contract_str in EXCHANGE_CONTRACTS {
        let spender = Address::from_str(contract_str)
            .map_err(|e| FalkeError::Wallet(format!("Invalid exchange address: {e}")))?;

        let allowance: U256 = usdc
            .allowance(signer.address(), spender)
            .call()
            .await
            .map_err(|e| {
                FalkeError::Wallet(format!("allowance() call failed for {contract_str}: {e}"))
            })?;

        if allowance == U256::ZERO {
            info!("Approving USDC for exchange contract {contract_str}...");
            let pending = usdc.approve(spender, U256::MAX).send().await.map_err(|e| {
                FalkeError::Wallet(format!("approve() send failed for {contract_str}: {e}"))
            })?;
            pending.get_receipt().await.map_err(|e| {
                FalkeError::Wallet(format!("approve() receipt failed for {contract_str}: {e}"))
            })?;
            info!("USDC approved for {contract_str}");
        } else {
            info!("USDC already approved for {contract_str} (allowance={allowance})");
        }
    }

    Ok(())
}
