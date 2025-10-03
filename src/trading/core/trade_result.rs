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
use spl_token::state::Mint;
use solana_program::program_pack::Pack;

/// Trade execution result containing actual transaction data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResult {
    /// Transaction signature
    pub signature: String,
    /// Actual token amount received (in UI format, e.g., 248.992717 for 6 decimals)
    /// For sell transactions, this will be negative (representing tokens sold)
    pub tokens_received: f64,
    /// Market entry price (SOL per token) - calculated from base trade amount excluding fees
    pub entry_price: f64,
    /// Total SOL amount spent including all fees (in SOL, not lamports)
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
    /// Token decimals (e.g., 6 for USDC, 9 for most tokens) - CRITICAL for accurate calculations
    pub token_decimals: u8,
    /// Post-trade token balance (remaining tokens after the transaction) - CRITICAL for account cleanup
    /// This is the actual balance left in the account after the sell, used to determine if cleanup is needed
    pub post_token_balance: Option<f64>,
}

impl TradeResult {
    /// Get token decimals from mint account
    #[allow(dead_code)]
    async fn get_token_decimals(
        rpc_client: &SolanaRpcClient,
        token_mint: &Pubkey,
    ) -> Result<u8> {
        println!("🔍 [MINT_DEBUG] Fetching decimals for token mint: {}", token_mint);
        
        let mint_account = rpc_client
            .get_account(token_mint)
            .await
            .map_err(|e| anyhow!("Failed to fetch mint account: {}", e))?;

        let mint_data = Mint::unpack(&mint_account.data)
            .map_err(|e| anyhow!("Failed to deserialize mint account: {}", e))?;

        println!("🔍 [MINT_DEBUG] Token mint {} has {} decimals on-chain", token_mint, mint_data.decimals);
        Ok(mint_data.decimals)
    }

    /// Calculate token amount in UI format from raw amount and decimals
    fn raw_amount_to_ui_amount(raw_amount: u64, decimals: u8) -> f64 {
        raw_amount as f64 / 10_f64.powi(decimals as i32)
    }
    
    /// Extract token decimals from transaction metadata as a backup verification method
    fn extract_decimals_from_transaction_meta(
        meta: &solana_transaction_status::UiTransactionStatusMeta,
        token_mint: &Pubkey,
        wallet_address: &Pubkey,
    ) -> Option<u8> {
        let token_mint_str = token_mint.to_string();
        let wallet_str = wallet_address.to_string();
        
        // Check post token balances for decimals info
        let post_token_balances = meta.post_token_balances.clone().unwrap_or(vec![]);
        for balance in post_token_balances {
            if balance.mint == token_mint_str && 
               balance.owner.as_ref() == Some(&wallet_str).into() {
                let decimals = balance.ui_token_amount.decimals;
                return Some(decimals);
            }
        }
        
        // Check pre token balances as fallback
        let pre_token_balances = meta.pre_token_balances.clone().unwrap_or(vec![]);
        for balance in pre_token_balances {
            if balance.mint == token_mint_str && 
               balance.owner.as_ref() == Some(&wallet_str).into() {
                let decimals = balance.ui_token_amount.decimals;
                return Some(decimals);
            }
        }
        
        None
    }
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

        // Get token decimals for accurate calculations
        // Get token decimals directly from transaction metadata (more reliable than RPC)
        let token_decimals = Self::extract_decimals_from_transaction_meta(&meta, token_mint, wallet_address)
            .unwrap_or_else(|| {
                6 // Default fallback
            });
        

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
                    .map(|pre| {
                        // 🔥 FIXED: Use ui_amount if available, otherwise calculate from raw amount
                        if let Some(ui_amount) = pre.ui_token_amount.ui_amount {
                            ui_amount
                        } else {
                            let raw_amount = pre.ui_token_amount.amount.parse::<u64>().unwrap_or(0);
                            Self::raw_amount_to_ui_amount(raw_amount, token_decimals)
                        }
                    })
                    .unwrap_or(0.0);

                // 🔥 FIXED: Use ui_amount if available, otherwise calculate from raw amount
                let post_amount = if let Some(ui_amount) = post_balance.ui_token_amount.ui_amount {
                    ui_amount
                } else {
                    let raw_amount = post_balance.ui_token_amount.amount.parse::<u64>().unwrap_or(0);
                    Self::raw_amount_to_ui_amount(raw_amount, token_decimals)
                };

                let token_delta = post_amount - pre_amount;

                if token_delta > 0.0 {
                    tokens_received = token_delta;
                    break;
                }
            }
        }

        // Calculate SOL spent from balance changes
        let pre_balances = &meta.pre_balances;
        let post_balances = &meta.post_balances;

        // 🎯 CRITICAL FIX: Find the user's wallet account by matching the address
        // Get account keys from the transaction to match wallet address
        let account_keys = match &transaction.transaction.transaction {
            solana_transaction_status::EncodedTransaction::Json(ui_tx) => {
                if let solana_transaction_status::UiMessage::Parsed(parsed_msg) = &ui_tx.message {
                    parsed_msg.account_keys.iter().map(|k| k.pubkey.clone()).collect::<Vec<String>>()
                } else {
                    vec![]
                }
            },
            _ => vec![]
        };

        // Find the index of the user's wallet in account_keys
        let wallet_index = account_keys.iter().position(|key| key == &wallet_str);
        
        if let Some(index) = wallet_index {
            // Found the user's wallet - get their SOL balance change
            if index < pre_balances.len() && index < post_balances.len() {
                let pre_balance_lamports = pre_balances[index];
                let post_balance_lamports = post_balances[index];
                let balance_delta_lamports = pre_balance_lamports as i64 - post_balance_lamports as i64;
                
                if balance_delta_lamports > 0 {
                    sol_spent = balance_delta_lamports as f64 / 1_000_000_000.0;
                    log::debug!("🔍 [TRADE_ANALYSIS] Found user's wallet at account index {} with SOL spent: {:.9}", 
                        index, sol_spent);
                }
            }
        } else {
            // Fallback: If we can't find the wallet in account keys, use the largest decrease
            log::warn!("⚠️ [TRADE_ANALYSIS] Could not find wallet {} in account keys, using fallback logic", wallet_address);
            let mut largest_decrease = 0i64;
            let mut best_index = 0usize;
            
            for (index, (&pre_balance, &post_balance)) in pre_balances.iter().zip(post_balances.iter()).enumerate() {
                let balance_delta = pre_balance as i64 - post_balance as i64;
                if balance_delta > largest_decrease {
                    largest_decrease = balance_delta;
                    best_index = index;
                }
            }
            
            if largest_decrease > 0 {
                sol_spent = largest_decrease as f64 / 1_000_000_000.0;
                log::debug!("🔍 [TRADE_ANALYSIS] Fallback: Found largest SOL decrease at account index {} with SOL spent: {:.6}", 
                    best_index, sol_spent);
            }
        }

        // Validate we found the expected data
        if tokens_received <= 0.0 {
            return Err(anyhow!("No token balance increase found for token {} and wallet {}", token_mint, wallet_address));
        }

        if sol_spent <= 0.0 {
            // Fallback to expected SOL amount if we can't calculate from balance changes
            sol_spent = expected_sol_spent;
            // If expected_sol_spent is also 0, try to estimate from transaction fees as minimum spent
            if sol_spent <= 0.0 && tokens_received > 0.0 {
                // Use network fees as minimum SOL spent + reasonable estimate for token purchase
                let base_network_fees = solana_fees.unwrap_or(5000) as f64 / 1_000_000_000.0; // ~0.000005 SOL
                let estimated_token_cost = tokens_received * 0.0001; // Conservative price estimate
                sol_spent = (base_network_fees + estimated_token_cost).max(0.001); // Minimum 0.001 SOL
                
                log::warn!("🚨 [TRADE_ANALYSIS] Could not determine actual SOL spent for transaction {}. Using estimated SOL spent: {:.6} SOL for {:.6} tokens (network fees: {:.6})", 
                    signature, sol_spent, tokens_received, base_network_fees);
            }
        }

        // Calculate market entry price using actual SOL spent (including slippage/fees)
        // This gives the true cost basis per token for accurate P&L and stop loss calculations
        let entry_price = if tokens_received > 0.0 { sol_spent / tokens_received } else { 0.0 };

        let analysis_duration_ms = analysis_start.elapsed().as_millis() as u64;
        
        // Debug logging for entry price calculation (using println to ensure visibility)
        println!("🔍 [TRADE_ANALYSIS] Signature: {} | SOL spent: {:.9} | Tokens received: {:.6} | Entry price: {:.10} | Token decimals: {}", 
            signature, sol_spent, tokens_received, entry_price, token_decimals);
        log::info!("🔍 [TRADE_ANALYSIS] Signature: {} | SOL spent: {:.9} | Tokens received: {:.6} | Entry price: {:.10} | Token decimals: {}", 
            signature, sol_spent, tokens_received, entry_price, token_decimals);

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
            token_decimals,  // 🔥 CRITICAL: Include actual token decimals in result
            post_token_balance: None, // Not relevant for buy transactions
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

        // Get token decimals for accurate calculations
        // Get token decimals directly from transaction metadata (more reliable than RPC)
        let token_decimals = Self::extract_decimals_from_transaction_meta(&meta, token_mint, wallet_address)
            .unwrap_or_else(|| {
                6 // Default fallback
            });
        

        // Analyze token balance changes
        let pre_token_balances = meta.pre_token_balances.unwrap_or(vec![]);
        let post_token_balances = meta.post_token_balances.unwrap_or(vec![]);

        // Find our token balance changes
        let token_mint_str = token_mint.to_string();
        let wallet_str = wallet_address.to_string();

        let mut tokens_sold = 0.0;
        let mut sol_received = 0.0;
        let mut post_token_balance = None;

        // Find pre-balance for our specific wallet and token mint
        let pre_balance = pre_token_balances
            .iter()
            .find(|balance| {
                balance.mint == token_mint_str && 
                balance.owner.as_ref() == Some(&wallet_str).into()
            });

        // Find post-balance for our specific wallet and token mint
        let post_balance = post_token_balances
            .iter()
            .find(|balance| {
                balance.mint == token_mint_str && 
                balance.owner.as_ref() == Some(&wallet_str).into()
            });

        // Calculate token amounts and capture post-balance
        if let (Some(pre), Some(post)) = (pre_balance, post_balance) {
            let pre_amount = if let Some(ui_amount) = pre.ui_token_amount.ui_amount {
                ui_amount
            } else {
                let raw_amount = pre.ui_token_amount.amount.parse::<u64>().unwrap_or(0);
                Self::raw_amount_to_ui_amount(raw_amount, token_decimals)
            };

            let post_amount = if let Some(ui_amount) = post.ui_token_amount.ui_amount {
                ui_amount
            } else {
                let raw_amount = post.ui_token_amount.amount.parse::<u64>().unwrap_or(0);
                Self::raw_amount_to_ui_amount(raw_amount, token_decimals)
            };

            // 🧹 CRITICAL: Always capture the post-balance for account cleanup decisions
            post_token_balance = Some(post_amount);

            let token_delta = pre_amount - post_amount;
            if token_delta > 0.0 {
                tokens_sold = token_delta;
            }
        }

        // Calculate SOL received from balance changes
        let pre_balances = &meta.pre_balances;
        let post_balances = &meta.post_balances;

        // 🎯 DEBUG: Log what we're looking for
        log::info!("🔍 [SELL_DEBUG] Looking for wallet: {}", wallet_str);
        log::info!("🔍 [SELL_DEBUG] Pre-balances count: {}, Post-balances count: {}", pre_balances.len(), post_balances.len());

        // 🎯 CRITICAL FIX: Find the user's wallet account by matching the address
        // Get account keys from the transaction to match wallet address
        let account_keys = match &transaction.transaction.transaction {
            solana_transaction_status::EncodedTransaction::Json(ui_tx) => {
                if let solana_transaction_status::UiMessage::Parsed(parsed_msg) = &ui_tx.message {
                    parsed_msg.account_keys.iter().map(|k| k.pubkey.clone()).collect::<Vec<String>>()
                } else {
                    log::warn!("🔍 [SELL_DEBUG] Transaction message is NOT Parsed");
                    vec![]
                }
            },
            _ => {
                log::warn!("🔍 [SELL_DEBUG] Transaction is NOT Json encoded");
                vec![]
            }
        };

        // 🎯 DEBUG: Log all account keys
        log::info!("🔍 [SELL_DEBUG] Found {} account keys:", account_keys.len());
        for (i, key) in account_keys.iter().enumerate() {
            log::info!("🔍 [SELL_DEBUG]   [{}]: {}", i, key);
            if i < pre_balances.len() && i < post_balances.len() {
                let delta = post_balances[i] as i64 - pre_balances[i] as i64;
                log::info!("🔍 [SELL_DEBUG]       Pre: {} lamports, Post: {} lamports, Delta: {} lamports ({:.9} SOL)", 
                    pre_balances[i], post_balances[i], delta, delta as f64 / 1_000_000_000.0);
            }
        }

        // Find the index of the user's wallet in account_keys
        let wallet_index = account_keys.iter().position(|key| key == &wallet_str);
        
        if let Some(index) = wallet_index {
            log::info!("✅ [SELL_DEBUG] FOUND user's wallet at index {}", index);
            // Found the user's wallet - get their SOL balance change
            if index < pre_balances.len() && index < post_balances.len() {
                let pre_balance_lamports = pre_balances[index];
                let post_balance_lamports = post_balances[index];
                let balance_delta_lamports = post_balance_lamports as i64 - pre_balance_lamports as i64;
                
                log::info!("🔍 [SELL_DEBUG] User wallet balance change: Pre={}, Post={}, Delta={} lamports ({:.9} SOL)", 
                    pre_balance_lamports, post_balance_lamports, balance_delta_lamports, balance_delta_lamports as f64 / 1_000_000_000.0);
                
                if balance_delta_lamports > 0 {
                    sol_received = balance_delta_lamports as f64 / 1_000_000_000.0;
                    log::info!("✅ [SELL_DEBUG] Setting sol_received to: {:.9} SOL", sol_received);
                } else {
                    log::warn!("⚠️ [SELL_DEBUG] Balance delta is NOT positive: {} - sol_received will remain 0", balance_delta_lamports);
                }
            } else {
                log::error!("❌ [SELL_DEBUG] Index {} is out of bounds for balance arrays!", index);
            }
        } else {
            // Fallback: If we can't find the wallet in account keys, use the largest increase
            log::warn!("⚠️ [SELL_DEBUG] Could NOT find wallet {} in account keys, using fallback logic", wallet_address);
            let mut largest_increase = 0i64;
            let mut best_index = 0usize;
            
            for (index, (&pre_balance, &post_balance)) in pre_balances.iter().zip(post_balances.iter()).enumerate() {
                let balance_delta = post_balance as i64 - pre_balance as i64;
                log::info!("🔍 [SELL_DEBUG] Fallback check [{}]: delta={} lamports", index, balance_delta);
                if balance_delta > largest_increase {
                    largest_increase = balance_delta;
                    best_index = index;
                }
            }
            
            if largest_increase > 0 {
                sol_received = largest_increase as f64 / 1_000_000_000.0;
                log::warn!("⚠️ [SELL_DEBUG] Using fallback: index {} with SOL received: {:.9}", best_index, sol_received);
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
            token_decimals,  // 🔥 CRITICAL: Include actual token decimals in result
            post_token_balance, // 🧹 CRITICAL: Actual remaining balance after sell for account cleanup
        })
    }
}

