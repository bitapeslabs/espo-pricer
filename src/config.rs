use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{fs, net::SocketAddr, path::Path};

fn default_rpc_host() -> String {
    "127.0.0.1".to_string()
}

fn default_rpc_port() -> u16 {
    8080
}

fn default_bitcoin_core_rpc_host() -> String {
    "127.0.0.1".to_string()
}

fn default_bitcoin_core_rpc_port() -> u16 {
    8332
}

fn default_binance_base_url() -> String {
    "https://data-api.binance.vision".to_string()
}

fn default_binance_symbol() -> String {
    "BTCUSDT".to_string()
}

fn default_poll_interval_ms() -> u64 {
    5_000
}

fn default_price_decimals() -> u32 {
    8
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_rpc_host")]
    pub rpc_host: String,
    #[serde(default = "default_rpc_port")]
    pub rpc_port: u16,

    #[serde(default)]
    pub binance_api_key: String,
    #[serde(default)]
    pub binance_api_secret: String,
    #[serde(default = "default_binance_base_url")]
    pub binance_base_url: String,
    #[serde(default = "default_binance_symbol")]
    pub binance_symbol: String,

    #[serde(alias = "metashrew_rpc")]
    pub metashrew_rpc_url: String,

    #[serde(alias = "adb_path")]
    pub db_path: String,
    pub min_height: u64,

    #[serde(default)]
    pub bitcoin_core_rpc_url: Option<String>,
    #[serde(default = "default_bitcoin_core_rpc_host")]
    pub bitcoin_core_rpc_host: String,
    #[serde(default = "default_bitcoin_core_rpc_port")]
    pub bitcoin_core_rpc_port: u16,
    pub bitcoin_core_rpc_username: String,
    pub bitcoin_core_rpc_password: String,

    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_price_decimals")]
    pub price_decimals: u32,
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn bind_addr(&self) -> Result<SocketAddr> {
        format!("{}:{}", self.rpc_host, self.rpc_port)
            .parse()
            .with_context(|| {
                format!(
                    "invalid rpc bind address {}:{}",
                    self.rpc_host, self.rpc_port
                )
            })
    }

    pub fn bitcoin_core_url(&self) -> String {
        match &self.bitcoin_core_rpc_url {
            Some(url) if !url.trim().is_empty() => url.trim().to_string(),
            _ => format!(
                "http://{}:{}",
                self.bitcoin_core_rpc_host, self.bitcoin_core_rpc_port
            ),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.metashrew_rpc_url.trim().is_empty() {
            bail!("metashrew_rpc_url must not be empty");
        }
        if self.db_path.trim().is_empty() {
            bail!("db_path must not be empty");
        }
        if self.bitcoin_core_rpc_username.trim().is_empty() {
            bail!("bitcoin_core_rpc_username must not be empty");
        }
        if self.bitcoin_core_rpc_password.is_empty() {
            bail!("bitcoin_core_rpc_password must not be empty");
        }
        if self.price_decimals > 18 {
            bail!("price_decimals must be <= 18");
        }
        Ok(())
    }
}
