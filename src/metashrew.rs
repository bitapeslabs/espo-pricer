use crate::jsonrpc::JsonRpcClient;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct MetashrewClient {
    rpc: JsonRpcClient,
}

impl MetashrewClient {
    pub fn new(url: String) -> Self {
        Self {
            rpc: JsonRpcClient::new(url),
        }
    }

    pub async fn tip_height(&self) -> Result<u64> {
        let value: Value = self.rpc.call("metashrew_height", json!([])).await?;
        parse_height_value(value)
    }
}

fn parse_height_value(value: Value) -> Result<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| anyhow!("metashrew_height returned a non-u64 number")),
        Value::String(raw) => raw
            .trim()
            .parse::<u64>()
            .map_err(|err| anyhow!("metashrew_height returned invalid height {raw:?}: {err}")),
        other => Err(anyhow!(
            "metashrew_height returned unsupported result type: {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metashrew_height_as_string_or_number() {
        assert_eq!(parse_height_value(json!("850000")).unwrap(), 850000);
        assert_eq!(parse_height_value(json!(850000)).unwrap(), 850000);
    }
}
