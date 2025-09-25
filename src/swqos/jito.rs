
use crate::swqos::common::{poll_transaction_confirmation, serialize_transaction_and_encode, FormatBase64VersionedTransaction};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::{sync::Arc, time::{Duration, Instant}};
use solana_transaction_status::UiTransactionEncoding;

use anyhow::Result;
use solana_sdk::{signature::Signature, transaction::VersionedTransaction};
use crate::swqos::{SwqosType, TradeType};
use crate::swqos::SwqosClientTrait;

use crate::{common::SolanaRpcClient, constants::swqos::JITO_TIP_ACCOUNTS};


pub struct JitoClient {
    pub endpoint: String,
    pub auth_token: String,
    pub rpc_client: Arc<SolanaRpcClient>,
    pub http_client: Client,
}

#[async_trait::async_trait]
impl SwqosClientTrait for JitoClient {
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<()> {
        self.send_transaction(trade_type, transaction).await
    }

    async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>) -> Result<()> {
        self.send_transactions(trade_type, transactions).await
    }

    fn get_tip_account(&self) -> Result<String> {
        if let Some(acc) = JITO_TIP_ACCOUNTS.choose(&mut rand::rng()) {
            Ok(acc.to_string())
        } else {
            Err(anyhow::anyhow!("no valid tip accounts found"))
        }
    }

    fn get_swqos_type(&self) -> SwqosType {
        SwqosType::Jito
    }
}

impl JitoClient {
    pub fn new(rpc_url: String, endpoint: String, auth_token: String) -> Self {
        let rpc_client = SolanaRpcClient::new(rpc_url);
        let http_client = Client::builder()
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(64)
            .tcp_keepalive(Some(Duration::from_secs(1200)))
            .http2_keep_alive_interval(Duration::from_secs(15))
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self { rpc_client: Arc::new(rpc_client), endpoint, auth_token, http_client }
    }

    pub async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<()> {
        let overall_start = Instant::now();
        let (content, signature) = serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64).await?;

        let request_body = serde_json::to_string(&json!({
            "id": 1,
            "jsonrpc": "2.0", 
            "method": "sendTransaction",
            "params": [
                content,
                {
                    "encoding": "base64"
                }
            ]
        }))?;

        let endpoint = if self.auth_token.is_empty() {
            format!("{}/api/v1/transactions", self.endpoint)
        } else {
            format!("{}/api/v1/transactions?uuid={}", self.endpoint, self.auth_token)
        };
        let response = if self.auth_token.is_empty() {
            self.http_client.post(&endpoint)
        } else {
            self.http_client.post(&endpoint)
                .header("x-jito-auth", &self.auth_token)
        };
        let response_text = response
            .body(request_body)
            .header("Content-Type", "application/json")
            .send()
            .await?
            .text()
            .await?;

        // Check submission result
        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_none() {
                if let Some(error) = response_json.get("error") {
                    println!("\x1b[31m❌ [Jito] {} submission failed: {} | Sig: {}\x1b[0m", trade_type, error, &signature.to_string()[..8]);
                    return Err(anyhow::anyhow!("Jito submission failed: {}", error));
                }
            }
        } else {
            println!("\x1b[31m❌ [Jito] {} submission failed: {} | Sig: {}\x1b[0m", trade_type, response_text, &signature.to_string()[..8]);
            return Err(anyhow::anyhow!("Jito submission failed: {}", response_text));
        }

        // Confirm transaction with retry logic for timeouts
        match self.confirm_transaction_with_retry(trade_type, signature, overall_start).await {
            Ok(_) => {
                // Success message is printed in confirm_transaction_with_retry
            },
            Err(e) => {
                return Err(e);
            },
        }

        Ok(())
    }

    pub async fn send_transactions(&self, trade_type: TradeType, transactions: &Vec<VersionedTransaction>) -> Result<()> {
        let start_time = Instant::now();
        let txs_base64 = transactions.iter().map(|tx| tx.to_base64_string()).collect::<Vec<String>>();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "sendBundle",
            "params": [
                txs_base64,
                { "encoding": "base64" }
            ],
            "id": 1,
        });

        let endpoint = if self.auth_token.is_empty() {
            format!("{}/api/v1/bundles", self.endpoint)
        } else {
            format!("{}/api/v1/bundles?uuid={}", self.endpoint, self.auth_token)
        };
        let response = if self.auth_token.is_empty() {
            self.http_client.post(&endpoint)
        } else {
            self.http_client.post(&endpoint)
                .header("x-jito-auth", &self.auth_token)
        };
        let response_text = response
            .body(body.to_string())
            .header("Content-Type", "application/json")
            .send()
            .await?
            .text()
            .await?;

        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_some() {
                println!(" jito {} submitted: {:?}", trade_type, start_time.elapsed());
            } else if let Some(_error) = response_json.get("error") {
                eprintln!(" jito {} submission failed: {:?}", trade_type, _error);
            }
        }

        Ok(())
    }

    /// Confirm transaction with retry logic for timeout errors
    async fn confirm_transaction_with_retry(
        &self, 
        trade_type: TradeType, 
        signature: Signature,
        overall_start: Instant
    ) -> Result<()> {
        let max_retries = 2; // As requested by user
        
        for attempt in 0..=max_retries {
            match poll_transaction_confirmation(&self.rpc_client, signature).await {
                Ok(_) => {
                    println!("\x1b[32m✅ [Jito] {} confirmed in {:?} | Sig: {}\x1b[0m", 
                        trade_type, overall_start.elapsed(), &signature.to_string()[..8]);
                    return Ok(());
                },
                Err(e) => {
                    let error_msg = e.to_string();
                    
                    // Check if this is a timeout error
                    if error_msg.contains("confirmation timed out") {
                        if attempt < max_retries {
                            println!("\x1b[33m⏰ [Jito] {} confirmation timed out on attempt {}, retrying... | Sig: {}\x1b[0m", 
                                trade_type, attempt + 1, &signature.to_string()[..8]);
                            
                            // Brief pause before retry
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            continue;
                        } else {
                            // All retries exhausted for timeout
                            println!("\x1b[31m❌ [Jito] {} confirmation failed after {} retries (all timeouts) in {:?} | Sig: {}\x1b[0m", 
                                trade_type, max_retries + 1, overall_start.elapsed(), &signature.to_string()[..8]);
                            return Err(anyhow::anyhow!("Transaction confirmation timed out after {} retries", max_retries + 1));
                        }
                    } else {
                        // Non-timeout error - don't retry, fail immediately
                        println!("\x1b[31m❌ [Jito] {} confirmation failed in {:?} | Sig: {} | Error: {}\x1b[0m", 
                            trade_type, overall_start.elapsed(), &signature.to_string()[..8], error_msg);
                        return Err(e);
                    }
                }
            }
        }
        
        // Should never reach here due to the loop logic above
        unreachable!()
    }
}