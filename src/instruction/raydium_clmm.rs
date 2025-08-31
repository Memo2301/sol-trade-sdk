use anyhow::{anyhow, Result};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}};

use crate::trading::core::{
    params::{BuyParams, SellParams},
    traits::{InstructionBuilder, ProtocolParams},
};

/// Raydium CLMM V1 instruction builder
pub struct RaydiumClmmInstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumClmmInstructionBuilder {
    async fn build_buy_instructions(
        &self,
        params: &BuyParams,
    ) -> Result<Vec<Instruction>, anyhow::Error> {
        let clmm_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumClmmParams>()
            .ok_or(anyhow!("Invalid parameters for Raydium CLMM"))?;

        let instruction = self.build_swap_instruction(
            &params.payer,
            &params.mint,
            params.sol_amount,
            clmm_params,
            true, // is_buy
        )?;
        
        Ok(vec![instruction])
    }

    async fn build_sell_instructions(
        &self,
        params: &SellParams,
    ) -> Result<Vec<Instruction>, anyhow::Error> {
        let clmm_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumClmmParams>()
            .ok_or(anyhow!("Invalid parameters for Raydium CLMM"))?;

        let instruction = self.build_swap_instruction(
            &params.payer,
            &params.mint,
            params.token_amount.unwrap_or(0),
            clmm_params,
            false, // is_sell
        )?;
        
        Ok(vec![instruction])
    }
}

impl RaydiumClmmInstructionBuilder {
    const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
    const SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];

    fn build_swap_instruction(
        &self,
        payer: &Keypair,
        _token_mint: &Pubkey,
        amount: u64,
        clmm_params: &RaydiumClmmParams,
        is_buy: bool,
    ) -> Result<Instruction> {
        // Determine input/output based on trade direction
        let (input_token_account, output_token_account, input_vault, output_vault) = if is_buy {
            // Buying token with SOL
            (
                clmm_params.payer_sol_account,
                clmm_params.payer_token_account,
                clmm_params.input_vault,
                clmm_params.output_vault,
            )
        } else {
            // Selling token for SOL  
            (
                clmm_params.payer_token_account,
                clmm_params.payer_sol_account,
                clmm_params.output_vault, // Swapped for sell
                clmm_params.input_vault,  // Swapped for sell
            )
        };

        // Build accounts array for CLMM V1
        let mut accounts = vec![
            AccountMeta::new(payer.pubkey(), true), // payer (signer)
            AccountMeta::new_readonly(clmm_params.amm_config, false), // amm_config
            AccountMeta::new(clmm_params.pool_state, false), // pool_state
            AccountMeta::new(input_token_account, false), // input_token_account
            AccountMeta::new(output_token_account, false), // output_token_account
            AccountMeta::new(input_vault, false), // input_vault
            AccountMeta::new(output_vault, false), // output_vault
            AccountMeta::new(clmm_params.observation_state, false), // observation_state
            AccountMeta::new_readonly(clmm_params.token_program, false), // token_program
        ];

        // Add tick arrays as remaining accounts
        for tick_array in &clmm_params.tick_arrays {
            accounts.push(AccountMeta::new(*tick_array, false));
        }

        // Build instruction data
        let mut data = Vec::new();
        data.extend_from_slice(&Self::SWAP_DISCRIMINATOR);
        data.extend_from_slice(&amount.to_le_bytes()); // amount
        data.extend_from_slice(&clmm_params.other_amount_threshold.to_le_bytes()); // other_amount_threshold
        data.extend_from_slice(&clmm_params.sqrt_price_limit_x64.to_le_bytes()); // sqrt_price_limit_x64
        data.push(if clmm_params.is_base_input { 1 } else { 0 }); // is_base_input

        Ok(Instruction {
            program_id: Self::PROGRAM_ID,
            accounts,
            data,
        })
    }
}

/// Raydium CLMM V2 instruction builder  
pub struct RaydiumClmmV2InstructionBuilder;

#[async_trait::async_trait]
impl InstructionBuilder for RaydiumClmmV2InstructionBuilder {
    async fn build_buy_instructions(
        &self,
        params: &BuyParams,
    ) -> Result<Vec<Instruction>, anyhow::Error> {
        let clmm_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumClmmV2Params>()
            .ok_or(anyhow!("Invalid parameters for Raydium CLMM V2"))?;

        let instruction = self.build_swap_instruction(
            &params.payer,
            &params.mint,
            params.sol_amount,
            clmm_params,
            true, // is_buy
        )?;
        
        Ok(vec![instruction])
    }

    async fn build_sell_instructions(
        &self,
        params: &SellParams,
    ) -> Result<Vec<Instruction>, anyhow::Error> {
        let clmm_params = params
            .protocol_params
            .as_any()
            .downcast_ref::<RaydiumClmmV2Params>()
            .ok_or(anyhow!("Invalid parameters for Raydium CLMM V2"))?;

        let instruction = self.build_swap_instruction(
            &params.payer,
            &params.mint,
            params.token_amount.unwrap_or(0),
            clmm_params,
            false, // is_sell
        )?;
        
        Ok(vec![instruction])
    }
}

impl RaydiumClmmV2InstructionBuilder {
    const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
    const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

    fn build_swap_instruction(
        &self,
        payer: &Keypair,
        _token_mint: &Pubkey,
        amount: u64,
        clmm_params: &RaydiumClmmV2Params,
        is_buy: bool,
    ) -> Result<Instruction> {
        // Determine input/output based on trade direction
        let (input_token_account, output_token_account, input_vault, output_vault) = if is_buy {
            // Buying token with SOL
            (
                clmm_params.payer_sol_account,
                clmm_params.payer_token_account,
                clmm_params.input_vault,
                clmm_params.output_vault,
            )
        } else {
            // Selling token for SOL  
            (
                clmm_params.payer_token_account,
                clmm_params.payer_sol_account,
                clmm_params.output_vault, // Swapped for sell
                clmm_params.input_vault,  // Swapped for sell
            )
        };

        // Build accounts array for CLMM V2
        let mut accounts = vec![
            AccountMeta::new(payer.pubkey(), true), // payer (signer)
            AccountMeta::new_readonly(clmm_params.amm_config, false), // amm_config
            AccountMeta::new(clmm_params.pool_state, false), // pool_state
            AccountMeta::new(input_token_account, false), // input_token_account
            AccountMeta::new(output_token_account, false), // output_token_account
            AccountMeta::new(input_vault, false), // input_vault
            AccountMeta::new(output_vault, false), // output_vault
            AccountMeta::new(clmm_params.observation_state, false), // observation_state
            AccountMeta::new_readonly(clmm_params.token_program, false), // token_program
            AccountMeta::new_readonly(clmm_params.token_program_2022, false), // token_program_2022
            AccountMeta::new_readonly(clmm_params.memo_program, false), // memo_program
            AccountMeta::new_readonly(clmm_params.input_vault_mint, false), // input_vault_mint
            AccountMeta::new_readonly(clmm_params.output_vault_mint, false), // output_vault_mint
        ];

        // Add tick arrays as remaining accounts
        for tick_array in &clmm_params.tick_arrays {
            accounts.push(AccountMeta::new(*tick_array, false));
        }

        // Build instruction data
        let mut data = Vec::new();
        data.extend_from_slice(&Self::SWAP_V2_DISCRIMINATOR);
        data.extend_from_slice(&amount.to_le_bytes()); // amount
        data.extend_from_slice(&clmm_params.other_amount_threshold.to_le_bytes()); // other_amount_threshold
        data.extend_from_slice(&clmm_params.sqrt_price_limit_x64.to_le_bytes()); // sqrt_price_limit_x64
        data.push(if clmm_params.is_base_input { 1 } else { 0 }); // is_base_input

        Ok(Instruction {
            program_id: Self::PROGRAM_ID,
            accounts,
            data,
        })
    }
}

/// Raydium CLMM V1 parameters
#[derive(Clone)]
pub struct RaydiumClmmParams {
    /// Core CLMM accounts
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub observation_state: Pubkey,
    /// Tick arrays for swap execution
    pub tick_arrays: Vec<Pubkey>,
    /// Token programs
    pub token_program: Pubkey,
    /// User token accounts
    pub payer_sol_account: Pubkey,
    pub payer_token_account: Pubkey,
    /// Instruction parameters
    pub other_amount_threshold: u64,
    pub sqrt_price_limit_x64: u128,
    pub is_base_input: bool,
}

impl ProtocolParams for RaydiumClmmParams {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}

/// Raydium CLMM V2 parameters
#[derive(Clone)]
pub struct RaydiumClmmV2Params {
    /// Core CLMM accounts
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub observation_state: Pubkey,
    /// Vault mint addresses (V2 specific)
    pub input_vault_mint: Pubkey,
    pub output_vault_mint: Pubkey,
    /// Tick arrays for swap execution
    pub tick_arrays: Vec<Pubkey>,
    /// Token programs (V2 includes token_program_2022)
    pub token_program: Pubkey,
    pub token_program_2022: Pubkey,
    pub memo_program: Pubkey,
    /// User token accounts
    pub payer_sol_account: Pubkey,
    pub payer_token_account: Pubkey,
    /// Instruction parameters
    pub other_amount_threshold: u64,
    pub sqrt_price_limit_x64: u128,
    pub is_base_input: bool,
}

impl ProtocolParams for RaydiumClmmV2Params {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProtocolParams> {
        Box::new(self.clone())
    }
}
