use crate::trading::middleware::traits::InstructionMiddleware;
use anyhow::Result;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::{Keypair, Signer}};
use solana_system_interface::instruction as system_instruction;
use std::sync::Arc;

/// Logging middleware - Records instruction information
#[derive(Clone)]
pub struct LoggingMiddleware;

impl InstructionMiddleware for LoggingMiddleware {
    fn name(&self) -> &'static str {
        "LoggingMiddleware"
    }

    fn process_protocol_instructions(
        &self,
        protocol_instructions: Vec<Instruction>,
        protocol_name: String,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        println!("-------------------[{}]-------------------", self.name());
        println!("process_protocol_instructions");
        println!("[{}] Instruction count: {}", self.name(), protocol_instructions.len());
        println!("[{}] Protocol name: {}\n", self.name(), protocol_name);
        println!("[{}] Is buy: {}", self.name(), is_buy);
        for (i, instruction) in protocol_instructions.iter().enumerate() {
            println!("Instruction {}:", i + 1);
            println!("{:?}\n", instruction);
        }
        Ok(protocol_instructions)
    }

    fn process_full_instructions(
        &self,
        full_instructions: Vec<Instruction>,
        protocol_name: String,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        println!("-------------------[{}]-------------------", self.name());
        println!("process_full_instructions");
        println!("[{}] Instruction count: {}", self.name(), full_instructions.len());
        println!("[{}] Protocol name: {}\n", self.name(), protocol_name);
        println!("[{}] Is buy: {}", self.name(), is_buy);
        for (i, instruction) in full_instructions.iter().enumerate() {
            println!("Instruction {}:", i + 1);
            println!("{:?}\n", instruction);
        }
        Ok(full_instructions)
    }

    fn clone_box(&self) -> Box<dyn InstructionMiddleware> {
        Box::new(self.clone())
    }
}

/// Custom fee middleware - Adds fee collection transfer before Jito tip
#[derive(Clone)]
pub struct CustomFeeMiddleware {
    pub fee_collection_wallet: Pubkey,
    pub fee_percentage: f64,
    pub minimum_fee_lamports: u64,
    pub payer: Arc<Keypair>,
    pub trade_amount_lamports: u64, // Pass trade amount for fee calculation
}

impl InstructionMiddleware for CustomFeeMiddleware {
    fn name(&self) -> &'static str {
        "CustomFeeMiddleware"
    }

    fn process_protocol_instructions(
        &self,
        protocol_instructions: Vec<Instruction>,
        _protocol_name: String,
        _is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        // Don't modify protocol instructions - only add to full instructions
        Ok(protocol_instructions)
    }

    fn process_full_instructions(
        &self,
        mut full_instructions: Vec<Instruction>,
        _protocol_name: String,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        // Only add fees for buy transactions
        if !is_buy {
            return Ok(full_instructions);
        }

        // Calculate fee amount
        let fee_amount = ((self.trade_amount_lamports as f64) * (self.fee_percentage / 100.0)) as u64;
        let final_fee_amount = fee_amount.max(self.minimum_fee_lamports);

        if final_fee_amount > 0 {
            // Create custom fee transfer instruction
            let fee_instruction = system_instruction::transfer(
                &self.payer.pubkey(),
                &self.fee_collection_wallet,
                final_fee_amount,
            );

            // CRITICAL: Insert custom fee BEFORE the last instruction (Jito tip)
            // Current order: [compute_budget, business_instructions, jito_tip]
            // Target order: [compute_budget, business_instructions, custom_fee, jito_tip]
            
            if let Some(last_instruction) = full_instructions.pop() {
                // Last instruction should be Jito tip - insert our fee before it
                full_instructions.push(fee_instruction);
                full_instructions.push(last_instruction); // Jito tip stays LAST
            } else {
                // Fallback: just add fee at end
                full_instructions.push(fee_instruction);
            }
        }

        Ok(full_instructions)
    }

    fn clone_box(&self) -> Box<dyn InstructionMiddleware> {
        Box::new(self.clone())
    }
}

impl CustomFeeMiddleware {
    pub fn new(
        fee_collection_wallet: Pubkey,
        fee_percentage: f64,
        minimum_fee_lamports: u64,
        payer: Arc<Keypair>,
        trade_amount_lamports: u64,
    ) -> Self {
        Self {
            fee_collection_wallet,
            fee_percentage,
            minimum_fee_lamports,
            payer,
            trade_amount_lamports,
        }
    }
}
