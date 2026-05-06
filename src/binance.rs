use crate::types::CachedCandle;
use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::Value;
use std::{
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

pub const FIFTEEN_MINUTES_SECS: u64 = 15 * 60;
pub const MAX_KLINES_PER_REQUEST: u16 = 1_000;

#[derive(Clone)]
pub struct BinanceClient {
    client: Client,
    base_url: String,
    api_key: String,
    _api_secret: String,
    symbol: String,
    price_decimals: u32,
}

impl BinanceClient {
    pub fn new(
        base_url: String,
        api_key: String,
        api_secret: String,
        symbol: String,
        price_decimals: u32,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            _api_secret: api_secret,
            symbol,
            price_decimals,
        }
    }

    pub async fn candles(
        &self,
        start_open_time: u64,
        end_open_time: u64,
    ) -> Result<Vec<CachedCandle>> {
        if start_open_time > end_open_time {
            return Ok(Vec::new());
        }

        let start_ms = seconds_to_millis(start_open_time)?;
        let end_ms = end_open_time
            .checked_add(FIFTEEN_MINUTES_SECS)
            .and_then(|ts| ts.checked_mul(1_000))
            .and_then(|ms| ms.checked_sub(1))
            .ok_or_else(|| anyhow!("candle end timestamp overflow"))?;

        let url = format!("{}/api/v3/klines", self.base_url);
        let mut request = self.client.get(url).query(&[
            ("symbol", self.symbol.as_str()),
            ("interval", "15m"),
            ("startTime", &start_ms.to_string()),
            ("endTime", &end_ms.to_string()),
            ("limit", &MAX_KLINES_PER_REQUEST.to_string()),
        ]);

        if !self.api_key.trim().is_empty() {
            request = request.header("X-MBX-APIKEY", self.api_key.trim());
        }

        let response = request
            .send()
            .await
            .context("failed to request Binance klines")?;
        let status = response.status();
        if !status.is_success() {
            let url = response.url().clone();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("failed to read error body: {err}"));
            bail!("Binance klines returned HTTP {status} for {url}: {body}");
        }

        let klines: Vec<Vec<Value>> = response
            .json()
            .await
            .context("failed to decode Binance kline response")?;
        klines
            .iter()
            .map(|kline| parse_kline(kline, self.price_decimals))
            .collect()
    }
}

fn parse_kline(kline: &[Value], price_decimals: u32) -> Result<CachedCandle> {
    let candle_open_time = kline
        .first()
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Binance kline missing open time"))?
        .checked_div(1_000)
        .ok_or_else(|| anyhow!("Binance kline open time overflow"))?;
    let price_scaled = kline
        .get(4)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Binance kline missing close price"))?
        .to_string();
    let price_raw = price_to_raw(&price_scaled, price_decimals)?;

    Ok(CachedCandle {
        candle_open_time,
        price_scaled,
        price_raw,
    })
}

pub fn normalize_to_15m(timestamp: u64) -> u64 {
    timestamp - (timestamp % FIFTEEN_MINUTES_SECS)
}

pub fn latest_candle_open_time() -> Result<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_secs();
    Ok(normalize_to_15m(now))
}

fn seconds_to_millis(timestamp: u64) -> Result<u64> {
    timestamp
        .checked_mul(1_000)
        .ok_or_else(|| anyhow!("timestamp overflow"))
}

pub fn price_to_raw(price: &str, decimals: u32) -> Result<String> {
    let multiplier = 10u64
        .checked_pow(decimals)
        .ok_or_else(|| anyhow!("price_decimals is too large"))?;
    let decimal = Decimal::from_str(price)
        .with_context(|| format!("failed to parse Binance close price {price:?}"))?;
    let scaled = decimal * Decimal::from(multiplier);
    if !scaled.fract().is_zero() {
        bail!("price {price:?} cannot be represented exactly with {decimals} decimals");
    }
    Ok(scaled.trunc().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_timestamp_to_15_minute_open() {
        assert_eq!(normalize_to_15m(0), 0);
        assert_eq!(normalize_to_15m(899), 0);
        assert_eq!(normalize_to_15m(900), 900);
        assert_eq!(normalize_to_15m(901), 900);
    }

    #[test]
    fn converts_decimal_price_to_raw_integer_string() {
        assert_eq!(price_to_raw("64123.45000000", 8).unwrap(), "6412345000000");
        assert_eq!(price_to_raw("1.23", 2).unwrap(), "123");
    }

    #[test]
    fn parses_binance_kline() {
        let candle = parse_kline(
            &[
                Value::from(900_000u64),
                Value::from("1.00"),
                Value::from("2.00"),
                Value::from("0.50"),
                Value::from("64123.45000000"),
            ],
            8,
        )
        .unwrap();
        assert_eq!(candle.candle_open_time, 900);
        assert_eq!(candle.price_raw, "6412345000000");
    }
}
