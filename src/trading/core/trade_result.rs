use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
};
use solana_transaction_status::UiTransactionEncoding;
use std::time::Instant;
use crate::common::SolanaRpcClient;

/// Trade execution result containing actual transaction data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    /// Transaction signature
    pub signature: String,
    /// Actual token amount received (in UI format, e.g., 248.992717 for 6 decimals)
    pub tokens_received: f64,
    /// Actual entry price (SOL per token)
    pub entry_price: f64,
    /// SOL amount spent (in SOL, not lamports)
    pub sol_spent: f64,
    /// Token mint address
    pub token_mint: String,
    /// Wallet address that executed the trade
    pub wallet_address: String,
    /// Time taken for transaction analysis (in milliseconds)
    pub analysis_duration_ms: u64,
}

impl TradeResult {
    /// Analyze a transaction to extract actual trade results
    /// 
    /// # Arguments
    /// 
    /// * `rpc_client` - RPC client for blockchain queries
    /// * `signature` - Transaction signature to analyze
    /// * `token_mint` - Expected token mint address
    /// * `wallet_address` - Wallet address that executed the trade
    /// * `expected_sol_spent` - Expected SOL amount spent (for validation)
    /// 
    /// # Returns
    /// 
    /// Returns `TradeResult` with actual trade data or error if analysis fails
    pub async fn analyze_transaction(
        rpc_client: &SolanaRpcClient,
        signature: &Signature,
        token_mint: &Pubkey,
        wallet_address: &Pubkey,
        expected_sol_spent: f64,
    ) -> Result<Self> {
        let analysis_start = Instant::now();
        
        println!("[TRADE_ANALYSIS] 🔍 Analyzing transaction: {}", signature);
        
        // Configure RPC request for transaction details
        let config = RpcTransactionConfig {
            commitment: Some(CommitmentConfig::confirmed()),
            encoding: Some(UiTransactionEncoding::JsonParsed),
            max_supported_transaction_version: Some(0),
        };

        // Fetch transaction details
        let transaction = rpc_client
            .get_transaction_with_config(signature, config)
            .await
            .map_err(|e| anyhow!("Failed to fetch transaction: {}", e))?;

        // Extract meta data
        let meta = transaction
            .transaction
            .meta
            .ok_or_else(|| anyhow!("Transaction meta not found"))?;

        // Check if transaction was successful
        if meta.err.is_some() {
            return Err(anyhow!("Transaction failed: {:?}", meta.err));
        }

        // Analyze token balance changes
        let pre_token_balances = meta.pre_token_balances.unwrap_or(vec![]);
        let post_token_balances = meta.post_token_balances.unwrap_or(vec![]);

        // Find our token balance changes
        let token_mint_str = token_mint.to_string();
        let wallet_str = wallet_address.to_string();

        let mut tokens_received = 0.0;
        let mut sol_spent = 0.0;

        // Find token balance change for our token mint and wallet
        for post_balance in &post_token_balances {
            if post_balance.mint == token_mint_str && post_balance.owner.as_ref() == Some(&wallet_str).into() {
                // Find corresponding pre-balance
                let pre_amount = pre_token_balances
                    .iter()
                    .find(|pre| {
                        pre.mint == token_mint_str && 
                        pre.owner.as_ref() == Some(&wallet_str).into() &&
                        pre.account_index == post_balance.account_index
                    })
                    .map(|pre| pre.ui_token_amount.ui_amount.unwrap_or(0.0))
                    .unwrap_or(0.0);

                let post_amount = post_balance.ui_token_amount.ui_amount.unwrap_or(0.0);
                let token_delta = post_amount - pre_amount;

                if token_delta > 0.0 {
                    tokens_received = token_delta;
                    println!("[TRADE_ANALYSIS] 📈 Token delta found: {:.6} tokens", tokens_received);
                    break;
                }
            }
        }

        // Calculate SOL spent from balance changes
        let pre_balances = &meta.pre_balances;
        let post_balances = &meta.post_balances;

        // Calculate SOL spent from balance changes 
        // Use first account (index 0) which is typically the signer/payer
        if pre_balances.len() > 0 && post_balances.len() > 0 {
            let pre_balance_lamports = pre_balances[0];
            let post_balance_lamports = post_balances[0];
            let balance_delta_lamports = pre_balance_lamports as i64 - post_balance_lamports as i64;
            
            if balance_delta_lamports > 0 {
                sol_spent = balance_delta_lamports as f64 / 1_000_000_000.0;
                println!("[TRADE_ANALYSIS] 💰 SOL spent: {:.9} SOL", sol_spent);
            }
        }

        // Validate we found the expected data
        if tokens_received <= 0.0 {
            return Err(anyhow!("No token balance increase found for token {} and wallet {}", token_mint, wallet_address));
        }

        if sol_spent <= 0.0 {
            // Fallback to expected SOL amount if we can't calculate from balance changes
            sol_spent = expected_sol_spent;
            println!("[TRADE_ANALYSIS] ⚠️ Using expected SOL amount: {:.6} SOL", sol_spent);
        }

        // Calculate actual entry price
        let entry_price = sol_spent / tokens_received;

        let analysis_duration_ms = analysis_start.elapsed().as_millis() as u64;

        println!("[TRADE_ANALYSIS] ✅ Analysis complete in {}ms: {:.6} tokens at {:.10} SOL per token", 
            analysis_duration_ms, tokens_received, entry_price);

        Ok(TradeResult {
            signature: signature.to_string(),
            tokens_received,
            entry_price,
            sol_spent,
            token_mint: token_mint_str,
            wallet_address: wallet_str,
            analysis_duration_ms,
        })
    }
}
