use crate::{
    common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed,
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::raydium_cpmm::{
        accounts, get_observation_state_pda, get_pool_pda, get_vault_account,
        SWAP_BASE_IN_DISCRIMINATOR,
    },
    trading::core::{
        params::{BuyParams, RaydiumCpmmParams, SellParams},
        traits::InstructionBuilder,
    },
    utils::calc::raydium_cpmm::compute_swap_amount,
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};

/// Instruction builder for RaydiumCpmm protocol
pub struct RaydiumCpmmInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumCpmmInstructionBuilder {
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
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            get_pool_pda(
                &protocol_params.amm_config,
                &protocol_params.base_mint,
                &protocol_params.quote_mint,
            )
            .unwrap()
        } else {
            protocol_params.pool_state
        };

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.base_mint == crate::constants::WSOL_TOKEN_ACCOUNT;
        let mint_token_program = if is_base_in {
            protocol_params.quote_token_program
        } else {
            protocol_params.base_token_program
        };

        let amount_in: u64 = params.sol_amount;
        let result = compute_swap_amount(
            protocol_params.base_reserve,
            protocol_params.quote_reserve,
            is_base_in,
            amount_in,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );
        let minimum_amount_out = result.min_amount_out;

        let wsol_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
            params.open_seed_optimize,
        );
        let mint_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.mint,
            &mint_token_program,
            params.open_seed_optimize,
        );

        let wsol_vault_account = get_vault_account(
            &pool_state,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            protocol_params,
            true,
        );
        let mint_vault_account =
            get_vault_account(&pool_state, &params.mint, protocol_params, false);

        let observation_state_account = if protocol_params.observation_state == Pubkey::default() {
            get_observation_state_pda(&pool_state).unwrap()
        } else {
            protocol_params.observation_state
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(6);

        if protocol_params.auto_handle_wsol {
            instructions
                .extend(crate::trading::common::handle_wsol(&params.payer.pubkey(), amount_in));
        }

        // Always create the output token account for CPMM buys (like backup version)
        // This prevents AccountNotInitialized errors
        instructions.extend(
            crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                &params.payer.pubkey(),
                &params.payer.pubkey(),
                &params.mint,
                &mint_token_program,
                params.open_seed_optimize,
            ),
        );

        // Create buy instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(pool_state, false),           // Pool State
            AccountMeta::new(wsol_token_account, false),   // Input Token Account
            AccountMeta::new(mint_token_account, false),   // Output Token Account
            AccountMeta::new(wsol_vault_account, false),   // Input Vault Account
            AccountMeta::new(mint_vault_account, false),   // Output Vault Account
            crate::constants::TOKEN_PROGRAM_META,          // Input Token Program (readonly)
            AccountMeta::new_readonly(mint_token_program, false), // Output Token Program (readonly)
            crate::constants::WSOL_TOKEN_ACCOUNT_META,     // Input token mint (readonly)
            AccountMeta::new_readonly(params.mint, false), // Output token mint (readonly)
            AccountMeta::new(observation_state_account, false), // Observation State Account
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[8..16].copy_from_slice(&amount_in.to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_CPMM,
            &data,
            accounts.to_vec(),
        ));

        if params.close_wsol_ata {
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
            .downcast_ref::<RaydiumCpmmParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for RaydiumCpmm"))?;

        if params.token_amount.is_none() || params.token_amount.unwrap_or(0) == 0 {
            return Err(anyhow!("Token amount is not set"));
        }

        let pool_state = if protocol_params.pool_state == Pubkey::default() {
            get_pool_pda(
                &protocol_params.amm_config,
                &protocol_params.base_mint,
                &protocol_params.quote_mint,
            )
            .unwrap()
        } else {
            protocol_params.pool_state
        };

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let is_base_in = protocol_params.base_mint == params.mint;
        let mint_token_program = if is_base_in {
            protocol_params.base_token_program
        } else {
            protocol_params.quote_token_program
        };

        let minimum_amount_out: u64 = compute_swap_amount(
            protocol_params.base_reserve,
            protocol_params.quote_reserve,
            is_base_in,
            params.token_amount.unwrap_or(0),
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        )
        .min_amount_out;

        let wsol_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
            params.open_seed_optimize,
        );
        let mint_token_account = get_associated_token_address_with_program_id_fast_use_seed(
            &params.payer.pubkey(),
            &params.mint,
            &mint_token_program,
            params.open_seed_optimize,
        );

        let wsol_vault_account = get_vault_account(
            &pool_state,
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            protocol_params,
            true,
        );
        let mint_vault_account =
            get_vault_account(&pool_state, &params.mint, protocol_params, false);

        let observation_state_account = if protocol_params.observation_state == Pubkey::default() {
            get_observation_state_pda(&pool_state).unwrap()
        } else {
            protocol_params.observation_state
        };

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(3);

        if protocol_params.auto_handle_wsol {
            instructions.extend(crate::trading::common::create_wsol_ata(&params.payer.pubkey()));
        }

        // Create sell instruction
        let accounts: [AccountMeta; 13] = [
            AccountMeta::new(params.payer.pubkey(), true), // Payer (signer)
            accounts::AUTHORITY_META,                      // Authority (readonly)
            AccountMeta::new(protocol_params.amm_config, false), // Amm Config (readonly)
            AccountMeta::new(pool_state, false),           // Pool State
            AccountMeta::new(mint_token_account, false),   // Input Token Account
            AccountMeta::new(wsol_token_account, false),   // Output Token Account
            AccountMeta::new(mint_vault_account, false),   // Input Vault Account
            AccountMeta::new(wsol_vault_account, false),   // Output Vault Account
            AccountMeta::new_readonly(mint_token_program, false), // Input Token Program (readonly)
            crate::constants::TOKEN_PROGRAM_META,          // Output Token Program (readonly)
            AccountMeta::new_readonly(params.mint, false), // Input token mint (readonly)
            crate::constants::WSOL_TOKEN_ACCOUNT_META,     // Output token mint (readonly)
            AccountMeta::new(observation_state_account, false), // Observation State Account
        ];
        // Create instruction data
        let mut data = [0u8; 24];
        data[..8].copy_from_slice(&SWAP_BASE_IN_DISCRIMINATOR);
        data[8..16].copy_from_slice(&params.token_amount.unwrap_or(0).to_le_bytes());
        data[16..24].copy_from_slice(&minimum_amount_out.to_le_bytes());

        instructions.push(Instruction::new_with_bytes(
            accounts::RAYDIUM_CPMM,
            &data,
            accounts.to_vec(),
        ));

        if protocol_params.auto_handle_wsol {
            // Close wSOL ATA account, reclaim rent (matches backup logic)
            instructions.extend(crate::trading::common::close_wsol(&params.payer.pubkey()));
        }

        Ok(instructions)
    }
}
