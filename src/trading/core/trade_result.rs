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
    /// For sell transactions, this will be negative (representing tokens sold)
    pub tokens_received: f64,
    /// Actual entry price (SOL per token)
    pub entry_price: f64,
    /// SOL amount spent (in SOL, not lamports)
    /// For sell transactions, this will be negative (representing SOL received)
    pub sol_spent: f64,
    /// Token mint address
    pub token_mint: String,
    /// Wallet address that executed the trade
    pub wallet_address: String,
    /// Time taken for transaction analysis (in milliseconds)
    pub analysis_duration_ms: u64,
    /// Optional: Profit/loss in absolute SOL amount (for sell transactions)
    pub profit_loss_absolute: Option<f64>,
    /// Optional: Profit/loss in percentage (for sell transactions)
    pub profit_loss_percentage: Option<f64>,
    /// Optional: Original entry price for profit calculation (for sell transactions)
    pub original_entry_price: Option<f64>,
    /// Slot number where the transaction was processed
    pub slot: Option<u64>,
    /// Solana network fees paid (in lamports)
    pub solana_fees: Option<u64>,
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
        
        // Transaction analysis started
        
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

        // Extract slot information
        let slot = transaction.slot;

        // Extract meta data
        let meta = transaction
            .transaction
            .meta
            .ok_or_else(|| anyhow!("Transaction meta not found"))?;

        // Extract Solana network fees
        let solana_fees = Some(meta.fee);

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
                    // Token delta found: {:.6} tokens
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
                // SOL spent: {:.9} SOL
            }
        }

        // Validate we found the expected data
        if tokens_received <= 0.0 {
            return Err(anyhow!("No token balance increase found for token {} and wallet {}", token_mint, wallet_address));
        }

        if sol_spent <= 0.0 {
            // Fallback to expected SOL amount if we can't calculate from balance changes
            sol_spent = expected_sol_spent;
            // Using expected SOL amount: {:.6} SOL
        }

        // Calculate actual entry price
        let entry_price = sol_spent / tokens_received;

        let analysis_duration_ms = analysis_start.elapsed().as_millis() as u64;

                // Analysis complete: {:.6} tokens at {:.10} SOL per token

        Ok(TradeResult {
            signature: signature.to_string(),
            tokens_received,
            entry_price,
            sol_spent,
            token_mint: token_mint_str,
            wallet_address: wallet_str,
            analysis_duration_ms,
            profit_loss_absolute: None,
            profit_loss_percentage: None,
            original_entry_price: None,
            slot: Some(slot),
            solana_fees,
        })
    }

    /// Analyze a sell transaction to extract actual trade results
    /// 
    /// # Arguments
    /// 
    /// * `rpc_client` - RPC client for blockchain queries
    /// * `signature` - Transaction signature to analyze
    /// * `token_mint` - Expected token mint address
    /// * `wallet_address` - Wallet address that executed the trade
    /// * `expected_tokens_sold` - Expected token amount sold
    /// * `original_entry_price` - Original entry price for profit calculation
    /// 
    /// # Returns
    /// 
    /// Returns `TradeResult` with actual sell trade data or error if analysis fails
    pub async fn analyze_sell_transaction(
        rpc_client: &SolanaRpcClient,
        signature: &Signature,
        token_mint: &Pubkey,
        wallet_address: &Pubkey,
        expected_tokens_sold: f64,
        original_entry_price: f64,
    ) -> Result<TradeResult> {
        let analysis_start = Instant::now();
        
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

        // Extract slot information
        let slot = transaction.slot;

        // Extract meta data
        let meta = transaction
            .transaction
            .meta
            .ok_or_else(|| anyhow!("Transaction meta not found"))?;

        // Extract Solana network fees
        let solana_fees = Some(meta.fee);

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

        let mut tokens_sold = 0.0;
        let mut sol_received = 0.0;

        // Find token balance change for our token mint and wallet
        for pre_balance in &pre_token_balances {
            if pre_balance.mint == token_mint_str && pre_balance.owner.as_ref() == Some(&wallet_str).into() {
                // Find corresponding post-balance
                let post_amount = post_token_balances
                    .iter()
                    .find(|post| {
                        post.mint == token_mint_str && 
                        post.owner.as_ref() == Some(&wallet_str).into() &&
                        post.account_index == pre_balance.account_index
                    })
                    .map(|post| post.ui_token_amount.ui_amount.unwrap_or(0.0))
                    .unwrap_or(0.0);

                let pre_amount = pre_balance.ui_token_amount.ui_amount.unwrap_or(0.0);
                let token_delta = pre_amount - post_amount;

                if token_delta > 0.0 {
                    tokens_sold = token_delta;
                    break;
                }
            }
        }

        // Calculate SOL received from balance changes
        let pre_balances = &meta.pre_balances;
        let post_balances = &meta.post_balances;

        // Calculate SOL received from balance changes 
        // Use first account (index 0) which is typically the signer/payer
        if pre_balances.len() > 0 && post_balances.len() > 0 {
            let pre_balance_lamports = pre_balances[0];
            let post_balance_lamports = post_balances[0];
            let balance_delta_lamports = post_balance_lamports as i64 - pre_balance_lamports as i64;
            
            if balance_delta_lamports > 0 {
                sol_received = balance_delta_lamports as f64 / 1_000_000_000.0;
            }
        }

        // Validate we found the expected data
        if tokens_sold <= 0.0 {
            tokens_sold = expected_tokens_sold; // Fallback to expected amount
        }

        if sol_received <= 0.0 {
            return Err(anyhow!("No SOL balance increase found for wallet {}", wallet_address));
        }

        // Calculate current price per token from this sell
        let current_price = sol_received / tokens_sold;

        // Calculate profit/loss
        let profit_loss_absolute = (current_price - original_entry_price) * tokens_sold;
        let profit_loss_percentage = if original_entry_price > 0.0 {
            ((current_price - original_entry_price) / original_entry_price) * 100.0
        } else {
            0.0
        };

        let analysis_duration_ms = analysis_start.elapsed().as_millis() as u64;

        Ok(TradeResult {
            signature: signature.to_string(),
            tokens_received: -tokens_sold, // Negative for sell (tokens sold)
            entry_price: current_price,    // Current sell price
            sol_spent: -sol_received,      // Negative for sell (SOL received)
            token_mint: token_mint_str,
            wallet_address: wallet_str,
            analysis_duration_ms,
            profit_loss_absolute: Some(profit_loss_absolute),
            profit_loss_percentage: Some(profit_loss_percentage),
            original_entry_price: Some(original_entry_price),
            slot: Some(slot),
            solana_fees,
        })
    }
}
