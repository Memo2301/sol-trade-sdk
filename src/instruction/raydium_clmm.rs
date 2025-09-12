use anyhow::{anyhow, Result};
use solana_program::instruction::{AccountMeta, Instruction};
use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}};
use solana_system_interface::instruction::transfer;
use spl_associated_token_account::{get_associated_token_address, instruction::create_associated_token_account_idempotent};
use spl_token;

use crate::{
    trading::core::{
        params::{BuyParams, SellParams, RaydiumClmmV2Params},
        traits::{InstructionBuilder, ProtocolParams},
    },
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

        let mut instructions = Vec::new();
        
        // 🔧 CRITICAL FIX: Create ATA initialization instructions and WSOL wrapping for buy
        
        // Derive user's WSOL ATA address
        let wsol_ata = get_associated_token_address(
            &params.payer.pubkey(),
            &spl_token::native_mint::ID
        );
        
        // Create WSOL ATA (idempotent) - for spending SOL
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &spl_token::native_mint::ID, // Use native mint, not the hardcoded account
            &spl_token::ID,
        ));
        
        // Transfer SOL to WSOL ATA for wrapping
        instructions.push(transfer(
            &params.payer.pubkey(),
            &wsol_ata,
            params.sol_amount,
        ));
        
        // Sync native to wrap SOL into WSOL
        instructions.push(spl_token::instruction::sync_native(
            &spl_token::ID,
            &wsol_ata,
        )?);
        
        // Create token mint ATA (idempotent)
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &params.mint,
            &clmm_params.output_token_program, // Use correct token program from params
        ));
        

        let swap_instruction = self.build_swap_instruction(
            &params.payer,
            &params.mint,
            params.sol_amount,
            clmm_params,
            true, // is_buy
        )?;
        
        instructions.push(swap_instruction);
        
        // 🔧 WSOL UNWRAPPING: Close WSOL ATA to unwrap any leftover WSOL back to SOL (matches backup)
        instructions.push(spl_token::instruction::close_account(
            &spl_token::ID,
            &wsol_ata,
            &params.payer.pubkey(), // destination for unwrapped SOL
            &params.payer.pubkey(), // authority
            &[],
        )?);
        
        
        Ok(instructions)
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

        let mut instructions = Vec::new();
        
        // 🔧 CRITICAL FIX: Create ATA initialization instructions for sell
        
        // Derive user's WSOL ATA address
        let wsol_ata = get_associated_token_address(
            &params.payer.pubkey(),
            &spl_token::native_mint::ID
        );
        
        // Create WSOL ATA (idempotent) - for receiving SOL
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &spl_token::native_mint::ID, // Use native mint, not hardcoded account
            &spl_token::ID,
        ));
        
        // Create token mint ATA (idempotent) - for selling tokens
        instructions.push(create_associated_token_account_idempotent(
            &params.payer.pubkey(),
            &params.payer.pubkey(),
            &params.mint,
            &clmm_params.input_token_program, // Use correct token program from params
        ));
        

        let swap_instruction = self.build_swap_instruction(
            &params.payer,
            &params.mint,
            params.token_amount.unwrap_or(0),
            clmm_params,
            false, // is_sell
        )?;
        
        instructions.push(swap_instruction);
        
        // 🔧 WSOL UNWRAPPING: Close WSOL ATA to unwrap WSOL back to SOL after sell (matches backup)
        instructions.push(spl_token::instruction::close_account(
            &spl_token::ID,
            &wsol_ata,
            &params.payer.pubkey(), // destination for unwrapped SOL
            &params.payer.pubkey(), // authority
            &[],
        )?);
        
        
        Ok(instructions)
    }
}

impl RaydiumClmmV2InstructionBuilder {
    const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
    const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

    fn build_swap_instruction(
        &self,
        payer: &Keypair,
        token_mint: &Pubkey,
        amount: u64,
        clmm_params: &RaydiumClmmV2Params,
        is_buy: bool,
    ) -> Result<Instruction> {
        // 🔧 CRITICAL FIX: Derive our own ATAs (not use original trader's accounts)
        let wsol_token_account = get_associated_token_address(
            &payer.pubkey(),
            &spl_token::native_mint::ID, // Use native mint ID for WSOL
        );
        let mint_token_account = get_associated_token_address(
            &payer.pubkey(),
            token_mint,
        );
        
        
        // Determine input/output based on trade direction (using our derived ATAs)
        let (input_token_account, output_token_account) = if is_buy {
            // Buying token with SOL
            (
                wsol_token_account,    // Our WSOL ATA
                mint_token_account,    // Our token ATA  
            )
        } else {
            // Selling token for SOL  
            (
                mint_token_account,    // Our token ATA
                wsol_token_account,    // Our WSOL ATA
            )
        };

        // Build accounts array for CLMM V2
        // Note: Vault swapping for sells is already done at parameter creation level
        let mut accounts = vec![
            AccountMeta::new(payer.pubkey(), true), // 0: payer (signer)
            AccountMeta::new_readonly(clmm_params.amm_config, false), // 1: amm_config
            AccountMeta::new(clmm_params.pool_state, false), // 2: pool_state
            AccountMeta::new(input_token_account, false), // 3: input_token_account
            AccountMeta::new(output_token_account, false), // 4: output_token_account
            AccountMeta::new(clmm_params.input_vault, false), // 5: input_vault (already swapped in params for sell)
            AccountMeta::new(clmm_params.output_vault, false), // 6: output_vault (already swapped in params for sell)
            AccountMeta::new(clmm_params.observation_state, false), // 7: observation_state
            AccountMeta::new_readonly(clmm_params.token_program, false), // 8: token_program
            AccountMeta::new_readonly(clmm_params.token_program_2022, false), // 9: token_program_2022
            AccountMeta::new_readonly(clmm_params.memo_program, false), // 10: memo_program
            AccountMeta::new_readonly(clmm_params.input_vault_mint, false), // 11: input_vault_mint (already swapped in params for sell)
            AccountMeta::new_readonly(clmm_params.output_vault_mint, false), // 12: output_vault_mint (already swapped in params for sell)
        ];

        // 🔧 ADJUSTMENT: Add tick arrays with position swap for sell executions (positions 15 & 16)
        if is_buy {
            // Buy: Add tick arrays normally
            for tick_array in &clmm_params.tick_arrays {
                accounts.push(AccountMeta::new(*tick_array, false));
            }
        } else {
            // Sell: Add tick arrays with positions 15 and 16 swapped
            for (i, _) in clmm_params.tick_arrays.iter().enumerate() {
                let account_position = 13 + i;
                let tick_array = if i == 2 && clmm_params.tick_arrays.len() > 3 {
                    // Position 15 (i=2): use tick_arrays[3] instead
                    println!("   Account {} (pos 15): {} -> {}", account_position, clmm_params.tick_arrays[2], clmm_params.tick_arrays[3]);
                    clmm_params.tick_arrays[3]
                } else if i == 3 && clmm_params.tick_arrays.len() > 2 {
                    // Position 16 (i=3): use tick_arrays[2] instead
                    println!("   Account {} (pos 16): {} -> {}", account_position, clmm_params.tick_arrays[3], clmm_params.tick_arrays[2]);
                    clmm_params.tick_arrays[2]
                } else {
                    // All other positions: use normal order
                    println!("   Account {} (pos {}): {} (unchanged)", account_position, account_position, clmm_params.tick_arrays[i]);
                    clmm_params.tick_arrays[i]
                };
                accounts.push(AccountMeta::new(tick_array, false));
            }
        }

        // Build instruction data
        let mut data = Vec::new();
        data.extend_from_slice(&Self::SWAP_V2_DISCRIMINATOR);
        data.extend_from_slice(&amount.to_le_bytes()); // amount
        data.extend_from_slice(&clmm_params.other_amount_threshold.to_le_bytes()); // other_amount_threshold
        data.extend_from_slice(&clmm_params.sqrt_price_limit_x64.to_le_bytes()); // sqrt_price_limit_x64
        data.push(1); // 🔧 FIX: Always true for both buy and sell in CLMM V2 (per copied transaction)

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
