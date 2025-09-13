use crate::{
    constants::trade::trade::DEFAULT_SLIPPAGE,
    instruction::utils::pumpswap::{
        accounts, fee_recipient_ata, get_user_volume_accumulator_pda, BUY_DISCRIMINATOR,
        SELL_DISCRIMINATOR,
    },
    trading::{
        core::{
            params::{BuyParams, PumpSwapParams, SellParams},
            traits::InstructionBuilder,
        },
    },
    utils::calc::pumpswap::{buy_quote_input_internal, sell_base_input_internal},
};
use anyhow::{anyhow, Result};
use solana_sdk::{
    instruction::Instruction,
    pubkey::Pubkey,
    signer::Signer,
};
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use solana_system_interface::instruction::transfer;

/// Instruction builder for PumpSwap protocol
pub struct PumpSwapInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for PumpSwapInstructionBuilder {
    async fn build_buy_instructions(&self, params: &BuyParams) -> Result<Vec<Instruction>> {
        // Get PumpSwap specific parameters
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;

        if params.sol_amount == 0 {
            return Err(anyhow!("Amount cannot be zero"));
        }

        // Build instructions based on whether account information is provided (like backup)
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let pool_base_token_reserves = protocol_params.pool_base_token_reserves;
        let pool_quote_token_reserves = protocol_params.pool_quote_token_reserves;

        self.build_buy_instructions_with_accounts(
            params,
            protocol_params.pool,
            base_mint,
            quote_mint,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            protocol_params.creator,
            protocol_params.auto_handle_wsol,
            protocol_params.fee_config,
            protocol_params.fee_program,
        )
        .await
    }

    async fn build_sell_instructions(&self, params: &SellParams) -> Result<Vec<Instruction>> {
        // Get PumpSwap specific parameters
        let protocol_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<PumpSwapParams>()
            .ok_or_else(|| anyhow!("Invalid protocol params for PumpSwap"))?;
        // Build instructions based on whether account information is provided (like backup)
        let base_mint = protocol_params.base_mint;
        let quote_mint = protocol_params.quote_mint;
        let pool_base_token_reserves = protocol_params.pool_base_token_reserves;
        let pool_quote_token_reserves = protocol_params.pool_quote_token_reserves;

        self.build_sell_instructions_with_accounts(
            params,
            protocol_params.pool,
            base_mint,
            quote_mint,
            pool_base_token_reserves,
            pool_quote_token_reserves,
            protocol_params.creator,
            protocol_params.auto_handle_wsol,
            protocol_params.fee_config,
            protocol_params.fee_program,
        )
        .await
    }
}

impl PumpSwapInstructionBuilder {
    /// Build buy instructions with provided account information (like backup)
    async fn build_buy_instructions_with_accounts(
        &self,
        params: &BuyParams,
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        creator: Pubkey,
        auto_handle_wsol: bool,
        fee_config: Pubkey,
        fee_program: Pubkey,
    ) -> Result<Vec<Instruction>> {
        // RPC validation like backup
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        let quote_mint_is_wsol = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT;

        let token_amount;
        let sol_amount;
        if quote_mint_is_wsol {
            let result = buy_quote_input_internal(
                params.sol_amount,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // base_amount_out
            token_amount = result.base;
            // max_quote_amount_in
            sol_amount = result.max_quote;
        } else {
            let result = sell_base_input_internal(
                params.sol_amount,
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // min_quote_amount_out
            token_amount = result.min_quote;
            // base_amount_in
            sol_amount = params.sol_amount;
        }

        // Create user token accounts (derive like backup)
        let user_base_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &base_mint,
        );
        let user_quote_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &quote_mint,
        );

        // Get pool token accounts (derive like backup) 
        let pool_base_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &base_mint,
                &crate::constants::TOKEN_PROGRAM,
            );

        let pool_quote_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &quote_mint,
                &crate::constants::TOKEN_PROGRAM,
            );

        let mut instructions = vec![];

        if auto_handle_wsol {
            // Handle wSOL (like backup)
            instructions.push(
                // Create wSOL ATA account if it doesn't exist
                create_associated_token_account_idempotent(
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &crate::constants::WSOL_TOKEN_ACCOUNT,
                    &crate::constants::TOKEN_PROGRAM,
                ),
            );
            instructions.push(
                // Transfer SOL to wSOL ATA account
                transfer(
                    &params.payer.pubkey(),
                    if quote_mint_is_wsol {
                        &user_quote_token_account
                    } else {
                        &user_base_token_account
                    },
                    sol_amount,
                ),
            );

            // Sync wSOL balance - CRITICAL for WSOL to work!
            instructions.push(
                spl_token::instruction::sync_native(
                    &crate::constants::TOKEN_PROGRAM,
                    if quote_mint_is_wsol {
                        &user_quote_token_account
                    } else {
                        &user_base_token_account
                    },
                )
                .unwrap(),
            );
        }

        // Create user's base token account (use hardcoded token program like backup)
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            if quote_mint_is_wsol { &base_mint } else { &quote_mint },
            &crate::constants::TOKEN_PROGRAM, // ✅ HARDCODED like backup
        ));

        // Derive creator vault accounts (like backup)
        let coin_creator_vault_ata = crate::instruction::utils::pumpswap::coin_creator_vault_ata(creator, quote_mint);
        let coin_creator_vault_authority = crate::instruction::utils::pumpswap::coin_creator_vault_authority(creator);
        let fee_recipient_ata = fee_recipient_ata(accounts::FEE_RECIPIENT, quote_mint);

        // Create buy instruction (like backup)
        let mut accounts = vec![
            solana_sdk::instruction::AccountMeta::new_readonly(pool, false), // pool_id (readonly)
            solana_sdk::instruction::AccountMeta::new(params.payer.pubkey(), true), // user (signer)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::GLOBAL_ACCOUNT, false), // global (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(base_mint, false), // base_mint (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(quote_mint, false), // quote_mint (readonly)
            solana_sdk::instruction::AccountMeta::new(user_base_token_account, false), // user_base_token_account
            solana_sdk::instruction::AccountMeta::new(user_quote_token_account, false), // user_quote_token_account
            solana_sdk::instruction::AccountMeta::new(pool_base_token_account, false), // pool_base_token_account
            solana_sdk::instruction::AccountMeta::new(pool_quote_token_account, false), // pool_quote_token_account
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::FEE_RECIPIENT, false), // fee_recipient (readonly)
            solana_sdk::instruction::AccountMeta::new(fee_recipient_ata, false), // fee_recipient_ata
            solana_sdk::instruction::AccountMeta::new_readonly(crate::constants::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly) - HARDCODED
            solana_sdk::instruction::AccountMeta::new_readonly(crate::constants::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly, duplicated as in JS) - HARDCODED
            solana_sdk::instruction::AccountMeta::new_readonly(crate::constants::SYSTEM_PROGRAM, false), // System Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(
                accounts::ASSOCIATED_TOKEN_PROGRAM,
                false,
            ), // ASSOCIATED_TOKEN_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::EVENT_AUTHORITY, false), // event_authority (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AMM_PROGRAM, false), // PUMP_AMM_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new(coin_creator_vault_ata, false), // coin_creator_vault_ata - DERIVED 
            solana_sdk::instruction::AccountMeta::new_readonly(coin_creator_vault_authority, false), // coin_creator_vault_authority (readonly) - DERIVED
        ];
        if quote_mint_is_wsol {
            accounts.push(solana_sdk::instruction::AccountMeta::new(
                crate::instruction::utils::pumpswap::get_global_volume_accumulator_pda().unwrap(),
                false,
            ));
            accounts.push(solana_sdk::instruction::AccountMeta::new(
                get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap(),
                false,
            ));
        }
        accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(fee_config, false));
        accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(fee_program, false));

        // Create instruction data
        let mut data = [0u8; 24];
        if quote_mint_is_wsol {
            data[..8].copy_from_slice(&BUY_DISCRIMINATOR);
            // base_amount_out
            data[8..16].copy_from_slice(&token_amount.to_le_bytes());
            // max_quote_amount_in
            data[16..24].copy_from_slice(&sol_amount.to_le_bytes());
        } else {
            data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
            // base_amount_in
            data[8..16].copy_from_slice(&sol_amount.to_le_bytes());
            // min_quote_amount_out
            data[16..24].copy_from_slice(&token_amount.to_le_bytes());
        }

        instructions.push(Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts,
            data: data.to_vec(),
        });
        
        if auto_handle_wsol {
            // Close wSOL ATA account, reclaim any leftover SOL after buy
            instructions.push(
                spl_token::instruction::close_account(
                    &crate::constants::TOKEN_PROGRAM,
                    if quote_mint_is_wsol {
                        &user_quote_token_account
                    } else {
                        &user_base_token_account
                    },
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &[&params.payer.pubkey()],
                )
                .unwrap(),
            );
        }
        
        Ok(instructions)
    }

    /// Build sell instructions with provided account information (like backup)
    async fn build_sell_instructions_with_accounts(
        &self,
        params: &SellParams,
        pool: Pubkey,
        base_mint: Pubkey,
        quote_mint: Pubkey,
        pool_base_token_reserves: u64,
        pool_quote_token_reserves: u64,
        creator: Pubkey,
        auto_handle_wsol: bool,
        fee_config: Pubkey,
        fee_program: Pubkey,
    ) -> Result<Vec<Instruction>> {
        // RPC validation like backup
        if params.rpc.is_none() {
            return Err(anyhow!("RPC is not set"));
        }
        if params.token_amount.is_none() {
            return Err(anyhow!("Token amount is not set"));
        }

        let quote_mint_is_wsol = quote_mint == crate::constants::WSOL_TOKEN_ACCOUNT;

        let token_amount;
        let sol_amount;
        if quote_mint_is_wsol {
            let result = sell_base_input_internal(
                params.token_amount.unwrap(),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // min_quote_amount_out
            sol_amount = result.min_quote;
            // base_amount_in
            token_amount = params.token_amount.unwrap();
        } else {
            let result = buy_quote_input_internal(
                params.token_amount.unwrap(),
                params.slippage_basis_points.unwrap_or(DEFAULT_SLIPPAGE),
                pool_base_token_reserves,
                pool_quote_token_reserves,
                &creator,
            )
            .unwrap();
            // base_amount_out
            sol_amount = result.base;
            token_amount = params.token_amount.unwrap();
        }

        // Create user token accounts (derive like backup)
        let user_base_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &base_mint,
        );
        let user_quote_token_account = spl_associated_token_account::get_associated_token_address(
            &params.payer.pubkey(),
            &quote_mint,
        );

        // Get pool token accounts (derive like backup)
        let pool_base_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &base_mint,
                &crate::constants::TOKEN_PROGRAM,
            );

        let pool_quote_token_account =
            spl_associated_token_account::get_associated_token_address_with_program_id(
                &pool,
                &quote_mint,
                &crate::constants::TOKEN_PROGRAM,
            );

        // Derive creator vault accounts (like backup)
        let coin_creator_vault_ata = crate::instruction::utils::pumpswap::coin_creator_vault_ata(creator, quote_mint);
        let coin_creator_vault_authority = crate::instruction::utils::pumpswap::coin_creator_vault_authority(creator);
        let fee_recipient_ata = fee_recipient_ata(accounts::FEE_RECIPIENT, quote_mint);

        let mut instructions = Vec::with_capacity(5);

        // Always create WSOL ATA for sells (like backup)
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &crate::constants::WSOL_TOKEN_ACCOUNT,
            &crate::constants::TOKEN_PROGRAM,
        ));

        // Create user's base token account (use hardcoded token program like backup)
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            if quote_mint_is_wsol { &base_mint } else { &quote_mint },
            &crate::constants::TOKEN_PROGRAM, // ✅ HARDCODED like backup
        ));

        // Create sell instruction (like backup)
        let mut accounts = vec![
            solana_sdk::instruction::AccountMeta::new_readonly(pool, false), // pool_id (readonly)
            solana_sdk::instruction::AccountMeta::new(params.payer.pubkey(), true), // user (signer)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::GLOBAL_ACCOUNT, false), // global (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(base_mint, false), // base_mint (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(quote_mint, false), // quote_mint (readonly)
            solana_sdk::instruction::AccountMeta::new(user_base_token_account, false), // user_base_token_account
            solana_sdk::instruction::AccountMeta::new(user_quote_token_account, false), // user_quote_token_account
            solana_sdk::instruction::AccountMeta::new(pool_base_token_account, false), // pool_base_token_account
            solana_sdk::instruction::AccountMeta::new(pool_quote_token_account, false), // pool_quote_token_account
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::FEE_RECIPIENT, false), // fee_recipient (readonly)
            solana_sdk::instruction::AccountMeta::new(fee_recipient_ata, false), // fee_recipient_ata
            solana_sdk::instruction::AccountMeta::new_readonly(crate::constants::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly) - HARDCODED
            solana_sdk::instruction::AccountMeta::new_readonly(crate::constants::TOKEN_PROGRAM, false), // TOKEN_PROGRAM_ID (readonly, duplicated as in JS) - HARDCODED
            solana_sdk::instruction::AccountMeta::new_readonly(crate::constants::SYSTEM_PROGRAM, false), // System Program (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(
                accounts::ASSOCIATED_TOKEN_PROGRAM,
                false,
            ), // ASSOCIATED_TOKEN_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::EVENT_AUTHORITY, false), // event_authority (readonly)
            solana_sdk::instruction::AccountMeta::new_readonly(accounts::AMM_PROGRAM, false), // PUMP_AMM_PROGRAM_ID (readonly)
            solana_sdk::instruction::AccountMeta::new(coin_creator_vault_ata, false), // coin_creator_vault_ata - DERIVED
            solana_sdk::instruction::AccountMeta::new_readonly(coin_creator_vault_authority, false), // coin_creator_vault_authority (readonly) - DERIVED
        ];
        if !quote_mint_is_wsol {
            accounts.push(solana_sdk::instruction::AccountMeta::new(
                crate::instruction::utils::pumpswap::get_global_volume_accumulator_pda().unwrap(),
                false,
            ));
            accounts.push(solana_sdk::instruction::AccountMeta::new(
                get_user_volume_accumulator_pda(&params.payer.pubkey()).unwrap(),
                false,
            ));
        }

        accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(fee_config, false));
        accounts.push(solana_sdk::instruction::AccountMeta::new_readonly(fee_program, false));

        // Create instruction data
        let mut data = [0u8; 24];
        if quote_mint_is_wsol {
            data[..8].copy_from_slice(&SELL_DISCRIMINATOR);
            // base_amount_in
            data[8..16].copy_from_slice(&token_amount.to_le_bytes());
            // min_quote_amount_out
            data[16..24].copy_from_slice(&sol_amount.to_le_bytes());
        } else {
            data[..8].copy_from_slice(&BUY_DISCRIMINATOR);
            // base_amount_out
            data[8..16].copy_from_slice(&sol_amount.to_le_bytes());
            // max_quote_amount_in
            data[16..24].copy_from_slice(&token_amount.to_le_bytes());
        }

        instructions.push(Instruction {
            program_id: accounts::AMM_PROGRAM,
            accounts,
            data: data.to_vec(),
        });
        
        if auto_handle_wsol {
            // Close wSOL ATA account after sell to convert WSOL back to SOL (like backup)
            instructions.push(
                spl_token::instruction::close_account(
                    &crate::constants::TOKEN_PROGRAM,
                    if quote_mint_is_wsol {
                        &user_quote_token_account
                    } else {
                        &user_base_token_account
                    },
                    &params.payer.pubkey(),
                    &params.payer.pubkey(),
                    &[&params.payer.pubkey()],
                )
                .unwrap(),
            );
        }
        
        Ok(instructions)
    }
}