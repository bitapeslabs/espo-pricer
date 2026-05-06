mod binance;
mod bitcoin_core;
mod config;
mod indexer;
mod jsonrpc;
mod metashrew;
mod server;
mod store;
mod types;

use crate::{
    binance::BinanceClient, bitcoin_core::BitcoinCoreClient, config::AppConfig, indexer::Indexer,
    metashrew::MetashrewClient, store::PriceStore,
};
use anyhow::Result;
use clap::Parser;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "espopricer")]
#[command(about = "Indexes BTC/USD 15m Binance closes by Bitcoin block height")]
struct Cli {
    #[arg(short, long, default_value = "config.json")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = Cli::parse();
    let config = AppConfig::load(&cli.config)?;
    let store = Arc::new(PriceStore::open(&config.db_path)?);

    let metashrew = MetashrewClient::new(config.metashrew_rpc_url.clone());
    let bitcoin_core = BitcoinCoreClient::new(
        config.bitcoin_core_url(),
        config.bitcoin_core_rpc_username.clone(),
        config.bitcoin_core_rpc_password.clone(),
    );
    let binance = BinanceClient::new(
        config.binance_base_url.clone(),
        config.binance_api_key.clone(),
        config.binance_api_secret.clone(),
        config.binance_symbol.clone(),
        config.price_decimals,
    );

    let indexer = Indexer::new(
        Arc::clone(&store),
        metashrew,
        bitcoin_core,
        binance,
        config.min_height,
        Duration::from_millis(config.poll_interval_ms),
    );
    tokio::spawn(async move {
        if let Err(err) = indexer.run().await {
            error!(error = ?err, "indexer exited");
        }
    });

    let addr = config.bind_addr()?;
    info!(%addr, "starting espopricer");

    tokio::select! {
        result = server::serve(addr, store) => result?,
        result = tokio::signal::ctrl_c() => {
            result?;
            info!("shutdown signal received");
        }
    }

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,espopricer=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
