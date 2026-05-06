use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone)]
pub struct JsonRpcClient {
    client: Client,
    url: String,
    auth: Option<(String, String)>,
    next_id: std::sync::Arc<AtomicU64>,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcEnvelope<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[allow(dead_code)]
    data: Option<Value>,
}

impl JsonRpcClient {
    pub fn new(url: String) -> Self {
        Self {
            client: Client::new(),
            url,
            auth: None,
            next_id: std::sync::Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn with_basic_auth(mut self, username: String, password: String) -> Self {
        self.auth = Some((username, password));
        self
    }

    pub async fn call<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let mut builder = self.client.post(&self.url).json(&request);
        if let Some((username, password)) = &self.auth {
            builder = builder.basic_auth(username, Some(password));
        }

        let response = builder
            .send()
            .await
            .with_context(|| format!("json-rpc request failed for method {method}"))?
            .error_for_status()
            .with_context(|| format!("json-rpc HTTP error for method {method}"))?;

        let envelope: JsonRpcEnvelope<T> = response
            .json()
            .await
            .with_context(|| format!("json-rpc response decode failed for method {method}"))?;

        if let Some(error) = envelope.error {
            bail!(
                "json-rpc error {} from {method}: {}",
                error.code,
                error.message
            );
        }

        envelope
            .result
            .ok_or_else(|| anyhow!("json-rpc response for {method} had neither result nor error"))
    }
}
