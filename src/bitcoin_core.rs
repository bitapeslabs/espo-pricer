use crate::jsonrpc::JsonRpcClient;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

#[derive(Clone)]
pub struct BitcoinCoreClient {
    rpc: JsonRpcClient,
}

#[derive(Debug, Deserialize)]
struct BlockHeader {
    time: u64,
}

impl BitcoinCoreClient {
    pub fn new(url: String, username: String, password: String) -> Self {
        Self {
            rpc: JsonRpcClient::new(url).with_basic_auth(username, password),
        }
    }

    pub async fn block_timestamp(&self, height: u64) -> Result<u64> {
        let hash: String = self
            .rpc
            .call("getblockhash", json!([height]))
            .await
            .with_context(|| format!("failed to get block hash for height {height}"))?;

        let header: BlockHeader = self
            .rpc
            .call("getblockheader", json!([hash, true]))
            .await
            .with_context(|| format!("failed to get block header for height {height}"))?;

        Ok(header.time)
    }
}
