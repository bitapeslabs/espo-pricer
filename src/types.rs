use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PricePoint {
    pub height: u64,
    pub timestamp: u64,
    pub candle_open_time: u64,
    pub price_scaled: String,
    pub price_raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedCandle {
    pub candle_open_time: u64,
    pub price_scaled: String,
    pub price_raw: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PriceResponse {
    pub price_scaled: String,
    pub price_raw: String,
}
