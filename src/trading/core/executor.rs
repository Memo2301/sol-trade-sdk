use anyhow::{anyhow, Result};
use solana_sdk::signature::Signer;
use std::sync::Arc;

use crate::trading::core::parallel::{buy_parallel_execute, sell_parallel_execute};

// Maximum loaded accounts data size limit for transactions (512 KB)
// This prevents MaxLoadedAccountsDataSizeExceeded errors in complex operations like Raydium CLMM
const MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT: u32 = 512 * 1024;

use super::{
    params::{BuyParams, BuyWithTipParams, SellParams, SellWithTipParams},
    timer::TradeTimer,
    trade_result::TradeResult,
    traits::{InstructionBuilder, TradeExecutor},
};

/// Generic trade executor implementation
pub struct GenericTradeExecutor {
    instruction_builder: Arc<dyn InstructionBuilder>,
    protocol_name: &'static str,
}

impl GenericTradeExecutor {
    pub fn new(
        instruction_builder: Arc<dyn InstructionBuilder>,
        protocol_name: &'static str,
    ) -> Self {
        Self { instruction_builder, protocol_name }
    }
}

#[async_trait::async_trait]
impl TradeExecutor for GenericTradeExecutor {
    async fn buy(
        &self,
        mut params: BuyParams,
        middleware_manager: Option<Arc<crate::trading::MiddlewareManager>>,
    ) -> Result<TradeResult> {
        if params.data_size_limit == 0 {
            params.data_size_limit = MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT;
        }
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        let mut timer = TradeTimer::new("Build buy transaction");
        
        // Override middleware_manager in params if provided
        if let Some(manager) = middleware_manager {
            params.middleware_manager = Some(manager);
        }
        
        // Build instructions
        let instructions = self.instruction_builder.build_buy_instructions(&params).await?;
        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    true,
                )?,
            None => instructions,
        };
        timer.stage("Build RPC transaction instructions");

        // Execute buy transaction
        let signature = buy_parallel_execute(params.clone(), final_instructions, self.protocol_name).await?;
        timer.stage("Transaction analysis");

        // Analyze transaction to get actual trade results
        let trade_result = TradeResult::analyze_transaction(
            &rpc,
            &signature,
            &params.mint,
            &params.payer.pubkey(),
            params.sol_amount as f64 / 1_000_000_000.0, // Convert lamports to SOL
        ).await?;

        timer.finish();
        Ok(trade_result)
    }

    async fn buy_with_tip(
        &self,
        params: BuyWithTipParams,
        middleware_manager: Option<Arc<crate::trading::MiddlewareManager>>,
    ) -> Result<TradeResult> {
        let mut timer = TradeTimer::new("Build buy transaction");

        // Store RPC for later analysis (CRITICAL: like backup version)
        let rpc_for_analysis = params.rpc.clone();

        // Convert to BuyParams for compatibility
        let buy_params = BuyParams {
            rpc: params.rpc,
            payer: params.payer.clone(),
            mint: params.mint,
            sol_amount: params.sol_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: Arc::new(params.priority_fee),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            data_size_limit: params.data_size_limit,
            wait_transaction_confirmed: true,
            protocol_params: params.protocol_params,
            open_seed_optimize: false,
            swqos_clients: params.swqos_clients.clone(),
            middleware_manager: middleware_manager,
            create_wsol_ata: false,
            close_wsol_ata: false,
            create_mint_ata: false,
        };

        // Build instructions
        let instructions = self.instruction_builder.build_buy_instructions(&buy_params).await?;
        let final_instructions = match &buy_params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    true,
                )?,
            None => instructions,
        };

        // Execute transactions in parallel to get signature
        let actual_signature = buy_parallel_execute(buy_params, final_instructions, self.protocol_name).await?;
        timer.stage("Transaction analysis");

        // Get RPC client for transaction analysis (CRITICAL: like backup version)
        let rpc = rpc_for_analysis.ok_or_else(|| anyhow!("RPC client not available for transaction analysis"))?;
        
        // Parse the signature returned from Jito execution (CRITICAL: like backup version)
        let signature = actual_signature;

        // Do REAL transaction analysis just like the standard buy method (CRITICAL: like backup version)
        let trade_result = TradeResult::analyze_transaction(
            &rpc,
            &signature,
            &params.mint,
            &params.payer.pubkey(),
            params.sol_amount as f64 / 1_000_000_000.0, // Convert lamports to SOL
        ).await?;

        timer.finish();
        Ok(trade_result)
    }

    async fn sell(
        &self,
        mut params: SellParams,
        middleware_manager: Option<Arc<crate::trading::MiddlewareManager>>,
    ) -> Result<TradeResult> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        let mut timer = TradeTimer::new("Build sell transaction");
        
        // Override middleware_manager in params if provided
        if let Some(manager) = middleware_manager {
            params.middleware_manager = Some(manager);
        }
        
        // Build instructions
        let instructions = self.instruction_builder.build_sell_instructions(&params).await?;
        let final_instructions = match &params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    false,
                )?,
            None => instructions,
        };
        timer.stage("Build RPC transaction instructions");

        // Execute sell transaction
        let signature = sell_parallel_execute(params.clone(), final_instructions, self.protocol_name).await?;
        timer.stage("Transaction analysis");

        // Analyze transaction to get actual trade results
        let trade_result = TradeResult::analyze_transaction(
            &rpc,
            &signature,
            &params.mint,
            &params.payer.pubkey(),
            0.0, // For sell, we analyze the tokens sold instead
        ).await?;

        timer.finish();
        Ok(trade_result)
    }

    async fn sell_with_tip(
        &self,
        params: SellWithTipParams,
        middleware_manager: Option<Arc<crate::trading::MiddlewareManager>>,
    ) -> Result<TradeResult> {
        let _timer = TradeTimer::new("Build sell transaction");

        // Convert to SellParams for compatibility
        let sell_params = SellParams {
            rpc: params.rpc,
            payer: params.payer.clone(),
            mint: params.mint,
            token_amount: params.token_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: Arc::new(params.priority_fee),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            wait_transaction_confirmed: true,
            with_tip: true,
            protocol_params: params.protocol_params,
            open_seed_optimize: false,
            swqos_clients: params.swqos_clients.clone(),
            middleware_manager: middleware_manager,
            create_wsol_ata: false,
            close_wsol_ata: false,
        };

        // Build instructions
        let instructions = self.instruction_builder.build_sell_instructions(&sell_params).await?;
        let final_instructions = match &sell_params.middleware_manager {
            Some(middleware_manager) => middleware_manager
                .apply_middlewares_process_protocol_instructions(
                    instructions,
                    self.protocol_name.to_string(),
                    false,
                )?,
            None => instructions,
        };

        // Execute transactions in parallel
        let signature = sell_parallel_execute(sell_params, final_instructions, self.protocol_name).await?;

        // For parallel execution, return estimated trade result
        let estimated_sol = (params.token_amount.unwrap_or(0) as f64 * 0.001) * 0.95; // Rough estimate
        let estimated_tokens = params.token_amount.unwrap_or(0) as f64;
        let estimated_price = if estimated_tokens > 0.0 {
            estimated_sol / estimated_tokens
        } else {
            0.0
        };

        let trade_result = TradeResult {
            signature: signature.to_string(),
            tokens_received: -estimated_tokens, // Negative for sell (tokens sold)
            entry_price: estimated_price,
            sol_spent: -estimated_sol, // Negative for sell (SOL received)
            token_mint: params.mint.to_string(),
            wallet_address: params.payer.pubkey().to_string(),
            analysis_duration_ms: 0,
            profit_loss_absolute: None,
            profit_loss_percentage: None,
            original_entry_price: None,
            slot: None,
            solana_fees: None,
            token_decimals: 6, // Default to 6 decimals
        };

        Ok(trade_result)
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}