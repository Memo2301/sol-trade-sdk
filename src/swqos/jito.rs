
use crate::swqos::common::{poll_transaction_confirmation, serialize_transaction_and_encode, FormatBase64VersionedTransaction};
use rand::seq::IndexedRandom;
use reqwest::Client;
use serde_json::json;
use std::{sync::Arc, time::Instant};

use std::time::Duration;
use solana_transaction_status::UiTransactionEncoding;

use anyhow::Result;
use solana_sdk::transaction::VersionedTransaction;
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
    async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<String> {
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

    pub async fn send_transaction(&self, trade_type: TradeType, transaction: &VersionedTransaction) -> Result<String> {
        let total_start = Instant::now();
        
        // Encode transaction
        let encode_start = Instant::now();
        let (content, signature) = serialize_transaction_and_encode(transaction, UiTransactionEncoding::Base64).await?;
        let encode_time = encode_start.elapsed();

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

        // Submit transaction
        let submit_start = Instant::now();
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

        let submit_time = submit_start.elapsed();
        let mut submit_success = false;

        if let Ok(response_json) = serde_json::from_str::<serde_json::Value>(&response_text) {
            if response_json.get("result").is_some() {
                submit_success = true;
            } else if let Some(_error) = response_json.get("error") {
                // Error will be logged in consolidated message
            }
        }

        // Confirm transaction
        let confirm_start = Instant::now();
        match poll_transaction_confirmation(&self.rpc_client, signature).await {
            Ok(_) => (),
            Err(_) => (),
        }
        let confirm_time = confirm_start.elapsed();
        let total_time = total_start.elapsed();

        // Consolidated one-line log
        println!("Jito {} execution: encode: {:.1}ms, submit: {:.1}ms, confirm: {:.1}ms, total: {:.1}ms [{}]", 
            trade_type, 
            encode_time.as_secs_f64() * 1000.0, 
            submit_time.as_secs_f64() * 1000.0, 
            confirm_time.as_secs_f64() * 1000.0, 
            total_time.as_secs_f64() * 1000.0,
            if submit_success { "SUCCESS" } else { "FAILED" });

        Ok(signature.to_string())
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
                println!("Jito {} bundle submitted: {:.1}ms [SUCCESS]", trade_type, start_time.elapsed().as_secs_f64() * 1000.0);
            } else if let Some(_error) = response_json.get("error") {
                println!("Jito {} bundle submitted: {:.1}ms [FAILED]", trade_type, start_time.elapsed().as_secs_f64() * 1000.0);
            }
        }

        Ok(())
    }
}