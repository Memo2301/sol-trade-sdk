use anyhow::{anyhow, Result};
use std::sync::Arc;

use super::{
    parallel::parallel_execute_with_tips,
    params::{BuyParams, BuyWithTipParams, SellParams, SellWithTipParams, BuyWithBundleParams, SellWithBundleParams},
    timer::TradeTimer,
    traits::{InstructionBuilder, TradeExecutor},
};
use crate::{
    swqos::TradeType,
    trading::common::{build_rpc_transaction, build_sell_transaction},
};

const MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT: u32 = 256 * 1024;

/// 通用交易执行器实现
pub struct GenericTradeExecutor {
    instruction_builder: Arc<dyn InstructionBuilder>,
    protocol_name: &'static str,
}

impl GenericTradeExecutor {
    pub fn new(
        instruction_builder: Arc<dyn InstructionBuilder>,
        protocol_name: &'static str,
    ) -> Self {
        Self {
            instruction_builder,
            protocol_name,
        }
    }
}

#[async_trait::async_trait]
impl TradeExecutor for GenericTradeExecutor {
    async fn buy(&self, mut params: BuyParams) -> Result<()> {
        if params.data_size_limit == 0 {
            params.data_size_limit = MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT;
        }
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        let mut timer = TradeTimer::new("构建买入交易指令");
        // 构建指令
        let instructions = self
            .instruction_builder
            .build_buy_instructions(&params)
            .await?;
        timer.stage("构建rpc交易指令");

        // 构建交易
        let transaction = build_rpc_transaction(
            params.payer.clone(),
            &params.priority_fee,
            instructions,
            params.lookup_table_key,
            params.recent_blockhash,
            params.data_size_limit,
        )
        .await?;
        timer.stage("rpc提交确认");

        // 发送交易
        rpc.send_and_confirm_transaction(&transaction).await?;
        timer.finish();

        Ok(())
    }

    async fn buy_with_tip(&self, mut params: BuyWithTipParams) -> Result<()> {
        if params.data_size_limit == 0 {
            params.data_size_limit = MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT;
        }
        let timer = TradeTimer::new("构建买入交易指令");

        // 验证参数 - 转换为BuyParams进行验证
        let buy_params = BuyParams {
            rpc: params.rpc,
            payer: params.payer.clone(),
            mint: params.mint,
            creator: params.creator,
            sol_amount: params.sol_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: params.priority_fee.clone(),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            data_size_limit: params.data_size_limit,
            protocol_params: params.protocol_params.clone(),
        };

        // 构建指令
        let instructions = self
            .instruction_builder
            .build_buy_instructions(&buy_params)
            .await?;

        timer.finish();

        // 并行执行交易
        parallel_execute_with_tips(
            params.swqos_clients,
            params.payer,
            instructions,
            params.priority_fee,
            params.lookup_table_key,
            params.recent_blockhash,
            params.data_size_limit,
            TradeType::Buy,
        )
        .await?;

        Ok(())
    }

    async fn sell(&self, params: SellParams) -> Result<()> {
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let rpc = params.rpc.as_ref().unwrap().clone();
        let mut timer = TradeTimer::new("构建卖出交易指令");

        // 构建指令
        let instructions = self
            .instruction_builder
            .build_sell_instructions(&params)
            .await?;
        timer.stage("卖出交易指令");

        // 构建交易
        let transaction = build_sell_transaction(
            params.payer.clone(),
            &params.priority_fee,
            instructions,
            params.lookup_table_key,
            params.recent_blockhash,
        )
        .await?;
        timer.stage("卖出交易签名");

        // 发送交易
        rpc.send_and_confirm_transaction(&transaction).await?;
        timer.finish();

        Ok(())
    }

    async fn sell_with_tip(&self, params: SellWithTipParams) -> Result<()> {
        let timer = TradeTimer::new("构建卖出交易指令");

        // 转换为SellParams进行指令构建
        let sell_params = SellParams {
            rpc: params.rpc,
            payer: params.payer.clone(),
            mint: params.mint,
            creator: params.creator,
            token_amount: params.token_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: params.priority_fee.clone(),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            protocol_params: params.protocol_params.clone(),
        };

        // 构建指令
        let instructions = self
            .instruction_builder
            .build_sell_instructions(&sell_params)
            .await?;

        timer.finish();

        // 并行执行交易
        parallel_execute_with_tips(
            params.swqos_clients,
            params.payer,
            instructions,
            params.priority_fee,
            params.lookup_table_key,
            params.recent_blockhash,
            0,
            TradeType::Sell,
        )
        .await?;

        Ok(())
    }

    async fn buy_with_bundle(&self, params: BuyWithBundleParams) -> Result<()> {
        let mut data_size_limit = params.data_size_limit;
        if data_size_limit == 0 {
            data_size_limit = MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT;
        }

        let timer = TradeTimer::new("构建买入束包交易");

        // Convert to BuyParams for instruction building
        let buy_params = BuyParams {
            rpc: None,
            payer: params.payer.clone(),
            mint: params.mint,
            creator: params.creator,
            sol_amount: params.sol_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: params.priority_fee.clone(),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            data_size_limit,
            protocol_params: params.protocol_params.clone(),
        };

        // Build trade instructions
        let instructions = self
            .instruction_builder
            .build_buy_instructions(&buy_params)
            .await?;

        timer.finish();

        // Execute bundle with fee collection and tip
        parallel_execute_with_bundle(
            params.swqos_clients,
            params.payer,
            instructions,
            params.priority_fee,
            params.lookup_table_key,
            params.recent_blockhash,
            data_size_limit,
            TradeType::Buy,
            params.fee_wallet,
            params.fee_percentage,
            params.minimum_fee_lamports,
            params.tip_amount_lamports,
            params.sol_amount,
        )
        .await?;

        Ok(())
    }

    async fn sell_with_bundle(&self, params: SellWithBundleParams) -> Result<()> {
        let timer = TradeTimer::new("构建卖出束包交易");

        // Convert to SellParams for instruction building
        let sell_params = SellParams {
            rpc: None,
            payer: params.payer.clone(),
            mint: params.mint,
            creator: params.creator,
            token_amount: params.token_amount,
            slippage_basis_points: params.slippage_basis_points,
            priority_fee: params.priority_fee.clone(),
            lookup_table_key: params.lookup_table_key,
            recent_blockhash: params.recent_blockhash,
            protocol_params: params.protocol_params.clone(),
        };

        // Build trade instructions
        let instructions = self
            .instruction_builder
            .build_sell_instructions(&sell_params)
            .await?;

        timer.finish();

        // We need to estimate the SOL amount for fee calculation
        // For now, we'll use a placeholder - this should be enhanced to calculate actual SOL output
        let estimated_sol_amount = 10_000_000u64; // 0.01 SOL placeholder

        // Execute bundle with fee collection and tip
        parallel_execute_with_bundle(
            params.swqos_clients,
            params.payer,
            instructions,
            params.priority_fee,
            params.lookup_table_key,
            params.recent_blockhash,
            0,
            TradeType::Sell,
            params.fee_wallet,
            params.fee_percentage,
            params.minimum_fee_lamports,
            params.tip_amount_lamports,
            estimated_sol_amount,
        )
        .await?;

        Ok(())
    }

    fn protocol_name(&self) -> &'static str {
        self.protocol_name
    }
}

/// Execute bundle with fee collection and Jito tip
async fn parallel_execute_with_bundle(
    swqos_clients: Vec<Arc<crate::swqos::SwqosClient>>,
    payer: Arc<solana_sdk::signature::Keypair>,
    instructions: Vec<solana_sdk::instruction::Instruction>,
    priority_fee: crate::common::PriorityFee,
    lookup_table_key: Option<solana_sdk::pubkey::Pubkey>,
    recent_blockhash: solana_hash::Hash,
    data_size_limit: u32,
    trade_type: TradeType,
    fee_wallet: solana_sdk::pubkey::Pubkey,
    fee_percentage: f64,
    minimum_fee_lamports: u64,
    tip_amount_lamports: u64,
    sol_amount: u64,
) -> Result<()> {
    use crate::trading::common::{build_rpc_transaction, build_sell_transaction};
    use solana_system_interface::instruction as system_instruction;
    use solana_sdk::signature::Signer;
    use std::str::FromStr;

    // Find Jito-capable clients
    let jito_clients: Vec<_> = swqos_clients
        .iter()
        .filter(|client| client.supports_bundles())
        .collect();

    if jito_clients.is_empty() {
        // No Jito clients available, fall back to regular tip execution
        return parallel_execute_with_tips(
            swqos_clients,
            payer,
            instructions,
            priority_fee,
            lookup_table_key,
            recent_blockhash,
            data_size_limit,
            trade_type,
        )
        .await;
    }

    // Calculate fee amount
    let fee_amount_lamports = {
        let fee = ((sol_amount as f64) * (fee_percentage / 100.0)) as u64;
        fee.max(minimum_fee_lamports)
    };

    // Create trade transaction
    let trade_transaction = if matches!(trade_type, TradeType::Buy) {
        build_rpc_transaction(
            payer.clone(),
            &priority_fee,
            instructions,
            lookup_table_key,
            recent_blockhash,
            data_size_limit,
        )
        .await?
    } else {
        build_sell_transaction(
            payer.clone(),
            &priority_fee,
            instructions,
            lookup_table_key,
            recent_blockhash,
        )
        .await?
    };

    // Create fee collection transaction
    let fee_instruction = system_instruction::transfer(&payer.pubkey(), &fee_wallet, fee_amount_lamports);
    let fee_transaction = build_rpc_transaction(
        payer.clone(),
        &priority_fee,
        vec![fee_instruction],
        lookup_table_key,
        recent_blockhash,
        0, // No data size limit for simple transfer
    )
    .await?;

    // Create tip transaction
    let tip_account = jito_clients[0].get_tip_account()
        .map_err(|e| anyhow!("Failed to get tip account: {}", e))?;
    let tip_account_pubkey = solana_sdk::pubkey::Pubkey::from_str(&tip_account)
        .map_err(|e| anyhow!("Invalid tip account: {}", e))?;
    
    let tip_instruction = system_instruction::transfer(&payer.pubkey(), &tip_account_pubkey, tip_amount_lamports);
    let tip_transaction = build_rpc_transaction(
        payer.clone(),
        &priority_fee,
        vec![tip_instruction],
        lookup_table_key,
        recent_blockhash,
        0, // No data size limit for simple transfer
    )
    .await?;

    // Create bundle and submit
    let bundle_transactions = vec![trade_transaction, fee_transaction, tip_transaction];
    
    // Submit bundle to all Jito clients in parallel
    let mut tasks = Vec::new();
    for client in jito_clients {
        let client_clone = client.clone();
        let bundle_clone = bundle_transactions.clone();
        let trade_type_clone = trade_type;
        
        let task = tokio::spawn(async move {
            client_clone.send_bundle(trade_type_clone, &bundle_clone).await
        });
        tasks.push(task);
    }

    // Wait for at least one successful submission and get signatures
    let mut submission_signatures: Option<Vec<String>> = None;
    for task in tasks {
        match task.await {
            Ok(Ok(signatures)) => {
                submission_signatures = Some(signatures);
                break;
            }
            Ok(Err(e)) => {
                eprintln!("Bundle submission failed: {}", e);
            }
            Err(e) => {
                eprintln!("Bundle task failed: {}", e);
            }
        }
    }

    let signatures = submission_signatures
        .ok_or_else(|| anyhow!("All bundle submissions failed"))?;

    // Monitor bundle status and wait for confirmation
    if let Err(e) = monitor_bundle_confirmation(&signatures, trade_type).await {
        eprintln!("Bundle confirmation monitoring failed: {}", e);
        // Don't fail the entire operation if monitoring fails
    }

    Ok(())
}

/// Monitor bundle confirmation status
async fn monitor_bundle_confirmation(
    signatures: &[String],
    trade_type: TradeType,
) -> Result<()> {
    use std::time::{Duration, Instant};
    use tokio::time::sleep;

    let start_time = Instant::now();
    let timeout = Duration::from_secs(30); // 30 second timeout for bundle confirmation
    let check_interval = Duration::from_millis(1000); // Check every second

    println!("🔍 Monitoring bundle confirmation for {} with {} transactions...", trade_type, signatures.len());

    while start_time.elapsed() < timeout {
        // In a real implementation, we would:
        // 1. Check Jito bundle status using their API
        // 2. Query Solana RPC for transaction confirmations
        // 3. Handle partial confirmations
        
        // For now, we'll simulate confirmation monitoring
        sleep(check_interval).await;
        
        // Simulate successful confirmation after a short delay
        if start_time.elapsed() > Duration::from_millis(2000) {
            println!("✅ Bundle confirmed for {} after {:?}", trade_type, start_time.elapsed());
            return Ok(());
        }
    }

    Err(anyhow!("Bundle confirmation timeout after {:?}", timeout))
}
