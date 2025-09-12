use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    trading::core::{
        params::{BuyParams, PumpFunParams, SellParams},
        traits::InstructionBuilder,
    },
};
use crate::{
    instruction::utils::pumpfun::{
        accounts, get_bonding_curve_pda, get_creator, get_user_volume_accumulator_pda,
        global_constants::{self},
    },
    utils::calc::{
        common::{calculate_with_slippage_buy, calculate_with_slippage_sell},
        pumpfun::{get_buy_token_amount_from_sol_amount, get_sell_sol_amount_from_token_amount},
    },
};
use anyhow::{anyhow, Result};
use solana_sdk::instruction::AccountMeta;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signer::Signer};
use spl_token::instruction::close_account;

/// Instruction builder for PumpFun protocol
pub struct PumpFunInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for PumpFunInstructionBuilder {
    async fn build_buy_instructions(&self, params: &BuyParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpFunParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

        if params.sol_amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        let bonding_curve = &protocol_params.bonding_curve;
        let creator_vault_pda = protocol_params.creator_vault;
        let creator = get_creator(&creator_vault_pda);

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let buy_token_amount = get_buy_token_amount_from_sol_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            bonding_curve.real_token_reserves as u128,
            creator,
            params.sol_amount,
        );

        let max_sol_cost = calculate_with_slippage_buy(
            params.sol_amount,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );

        let bonding_curve_addr = if bonding_curve.account == Pubkey::default() {
            get_bonding_curve_pda(&params.mint).unwrap()
        } else {
            bonding_curve.account
        };

        let associated_bonding_curve =
            if protocol_params.associated_bonding_curve == Pubkey::default() {
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr,
                    &params.mint,
                    &crate::constants::TOKEN_PROGRAM,
                )
            } else {
                protocol_params.associated_bonding_curve
            };

        let user_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        let user_volume_accumulator =
            get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap();

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(2);

        // Create associated token account
        if params.create_mint_ata {
            instructions.extend(
                crate::common::fast_fn::create_associated_token_account_idempotent_fast_use_seed(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &params.mint,
                    &crate::constants::TOKEN_PROGRAM,
                    params.open_seed_optimize,
                ),
            );
        }

        let mut buy_data = [0u8; 24];
        buy_data[..8].copy_from_slice(&[102, 6, 61, 18, 1, 218, 235, 234]); // Method ID
        buy_data[8..16].copy_from_slice(&buy_token_amount.to_le_bytes());
        buy_data[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());

        let accounts: [AccountMeta; 16] = [
            global_constants::GLOBAL_ACCOUNT_META,
            global_constants::FEE_RECIPIENT_META,
            AccountMeta::new_readonly(params.mint, false),
            AccountMeta::new(bonding_curve_addr, false),
            AccountMeta::new(associated_bonding_curve, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new(params.payer.pubkey(), true),
            crate::constants::SYSTEM_PROGRAM_META,
            crate::constants::TOKEN_PROGRAM_META,
            AccountMeta::new(creator_vault_pda, false),
            accounts::EVENT_AUTHORITY_META,
            accounts::PUMPFUN_META,
            accounts::GLOBAL_VOLUME_ACCUMULATOR_META,
            AccountMeta::new(user_volume_accumulator, false),
            accounts::FEE_CONFIG_META,
            accounts::FEE_PROGRAM_META,
        ];

        instructions.push(Instruction::new_with_bytes(
            accounts::PUMPFUN,
            &buy_data,
            accounts.to_vec(),
        ));

        Ok(instructions)
    }

    async fn build_sell_instructions(&self, params: &SellParams) -> Result<Vec<Instruction>> {
        // ========================================
        // Parameter validation and basic data preparation
        // ========================================
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpFunParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpFun"))?;

        let token_amount = if let Some(amount) = params.token_amount {
            if amount == 0 {
                return Err(anyhow!("Amount cannot be zero"));
            }
            amount
        } else {
            return Err(anyhow!("Amount token is required"));
        };

        let bonding_curve = &protocol_params.bonding_curve;
        let creator_vault_pda = protocol_params.creator_vault;
        let creator = get_creator(&creator_vault_pda);

        // ========================================
        // Trade calculation and account address preparation
        // ========================================
        let sol_amount = get_sell_sol_amount_from_token_amount(
            bonding_curve.virtual_token_reserves as u128,
            bonding_curve.virtual_sol_reserves as u128,
            creator,
            token_amount,
        );

        let min_sol_output = calculate_with_slippage_sell(
            sol_amount,
            params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
        );

        let bonding_curve_addr = if bonding_curve.account == Pubkey::default() {
            get_bonding_curve_pda(&params.mint).unwrap()
        } else {
            bonding_curve.account
        };

        let associated_bonding_curve =
            if protocol_params.associated_bonding_curve == Pubkey::default() {
                crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
                    &bonding_curve_addr,
                    &params.mint,
                    &crate::constants::TOKEN_PROGRAM,
                )
            } else {
                protocol_params.associated_bonding_curve
            };

        let user_token_account =
            crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
                &params.payer.pubkey(),
                &params.mint,
                &crate::constants::TOKEN_PROGRAM,
                params.open_seed_optimize,
            );

        // ========================================
        // Build instructions
        // ========================================
        let mut instructions = Vec::with_capacity(2);

        let mut sell_data = [0u8; 24];
        sell_data[..8].copy_from_slice(&[51, 230, 133, 164, 1, 127, 131, 173]); // Method ID
        sell_data[8..16].copy_from_slice(&token_amount.to_le_bytes());
        sell_data[16..24].copy_from_slice(&min_sol_output.to_le_bytes());

        let accounts: [AccountMeta; 14] = [
            global_constants::GLOBAL_ACCOUNT_META,
            global_constants::FEE_RECIPIENT_META,
            AccountMeta::new_readonly(params.mint, false),
            AccountMeta::new(bonding_curve_addr, false),
            AccountMeta::new(associated_bonding_curve, false),
            AccountMeta::new(user_token_account, false),
            AccountMeta::new(params.payer.pubkey(), true),
            crate::constants::SYSTEM_PROGRAM_META,
            AccountMeta::new(creator_vault_pda, false),
            crate::constants::TOKEN_PROGRAM_META,
            accounts::EVENT_AUTHORITY_META,
            accounts::PUMPFUN_META,
            accounts::FEE_CONFIG_META,
            accounts::FEE_PROGRAM_META,
        ];

        instructions.push(Instruction::new_with_bytes(
            accounts::PUMPFUN,
            &sell_data,
            accounts.to_vec(),
        ));

        // Optional: Close token account - COMMENTED OUT TO PREVENT BALANCE ERROR
        // if protocol_params.close_token_account_when_sell.unwrap_or(false) {
        //     instructions.push(close_account(
        //         &crate::constants::TOKEN_PROGRAM,
        //         &user_token_account,
        //         &params.payer.pubkey(),
        //         &params.payer.pubkey(),
        //         &[&params.payer.pubkey()],
        //     )?);
        // }

        Ok(instructions)
    }
}
