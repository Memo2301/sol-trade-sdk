use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::raydium_amm_v4::{accounts, SWAP_BASE_IN_DISCRIMINATOR},
    trading::core::{
        params::{BuyParams, RaydiumAmmV4Params, SellParams},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_amm_v4::compute_swap_amount,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumAmmV4InstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumAmmV4InstructionBuilder {
    async fn build_buy_instructions(&self, params: &BuyParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        if params.sol_amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumAmmV4Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.coin_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let amount_in: u64 = params.sol_amount;
        let swap_result = compute_swap_amount(
            protocol_params.coin_reserve,
            protocol_params.pc_reserve,
            is_base_in,
            amount_in,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );
        let minimum_amount_out = swap_result.min_amount_out;

        let user_source_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &crate::constants::WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        let user_destination_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if protocol_params.auto_handle_wsol {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), amount_in));
        }

        // ALWAYS create destination token ATA (like backup does)
        instructions.extend(
            crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &params.mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            ),
        );

        // Create buy instruction with proper account addresses
        let accounts: [AccountMeta; 17] = [
            crate::constants::TOKEN_PROGRAM_META, // Token Program (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm
            accounts::AUTHORITY_META,             // Authority (readonly)
            AccountMeta::new(protocol_params.open_orders, false), // Amm Open Orders
            AccountMeta::new(protocol_params.token_coin, false), // Pool Coin Token Account
            AccountMeta::new(protocol_params.token_pc, false), // Pool Pc Token Account
            AccountMeta::new_readonly(protocol_params.serum_dex, false), // Serum Program
            AccountMeta::new(protocol_params.market, false), // Serum Market
            AccountMeta::new(protocol_params.market, false), // Serum Bids (should be from market data)
            AccountMeta::new(protocol_params.market, false), // Serum Asks (should be from market data)
            AccountMeta::new(protocol_params.market, false), // Serum Event Queue (should be from market data)
            AccountMeta::new(protocol_params.token_coin, false), // Serum Coin Vault Account
            AccountMeta::new(protocol_params.token_pc, false), // Serum Pc Vault Account
            AccountMeta::new_readonly(protocol_params.market, false), // Serum Vault Signer
            AccountMeta::new(user_source_token_account, false), // User Source Token Account
            AccountMeta::new(user_destination_token_account, false), // User Destination Token Account
            AccountMeta::new(params.payer.pubkey(), true),           // User Source Owner
        ];
        // Create instruction data
        let mut data = [0u8; 17];
        data[..1].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[1..9].copy_from_slice(&amount_in.to_le_bytes());
        data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_AMM_V4,
            &data,
            accounts.to_vec(),
        ));

        if protocol_params.auto_handle_wsol {
            // Close wSOL ATA account, reclaim rent
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SellParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumAmmV4Params>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        if params.token_amount.is_none() || params.token_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.pc_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let swap_result = compute_swap_amount(
            protocol_params.coin_reserve,
            protocol_params.pc_reserve,
            is_base_in,
            params.token_amount.unwrap_or(0),
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );
        let minimum_amount_out = swap_result.min_amount_out;

        let user_source_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );
        let user_destination_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &crate::constants::WSOL_TOKEN_ACCOUNT,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(3);

        if protocol_params.auto_handle_wsol {
            instructions.extend(crate::trading::common::create_wsol_ata(&params.payer.pubkey()));
        }

        // Create sell instruction with proper account addresses
        let accounts: [AccountMeta; 17] = [
            crate::constants::TOKEN_PROGRAM_META, // Token Program (readonly)
            AccountMeta::new(protocol_params.amm, false), // Amm
            accounts::AUTHORITY_META,             // Authority (readonly)
            AccountMeta::new(protocol_params.open_orders, false), // Amm Open Orders
            AccountMeta::new(protocol_params.token_coin, false), // Pool Coin Token Account
            AccountMeta::new(protocol_params.token_pc, false), // Pool Pc Token Account
            AccountMeta::new_readonly(protocol_params.serum_dex, false), // Serum Program
            AccountMeta::new(protocol_params.market, false), // Serum Market
            AccountMeta::new(protocol_params.market, false), // Serum Bids (should be from market data)
            AccountMeta::new(protocol_params.market, false), // Serum Asks (should be from market data)
            AccountMeta::new(protocol_params.market, false), // Serum Event Queue (should be from market data)
            AccountMeta::new(protocol_params.token_coin, false), // Serum Coin Vault Account
            AccountMeta::new(protocol_params.token_pc, false), // Serum Pc Vault Account
            AccountMeta::new_readonly(protocol_params.market, false), // Serum Vault Signer
            AccountMeta::new(user_source_token_account, false), // User Source Token Account
            AccountMeta::new(user_destination_token_account, false), // User Destination Token Account
            AccountMeta::new(params.payer.pubkey(), true),           // User Source Owner
        ];
        // Create instruction data
        let mut data = [0u8; 17];
        data[..1].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[1..9].copy_from_slice(&params.token_amount.unwrap_or(0).to_le_bytes());
        data[9..17].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_AMM_V4,
            &data,
            accounts.to_vec(),
        ));

        if protocol_params.auto_handle_wsol {
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }
}
