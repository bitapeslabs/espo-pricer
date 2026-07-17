use crate::types::{CachedCandle, PricePoint};
use anyhow::{Context, Result, anyhow};
use rocksdb::{DB, Direction, IteratorMode, Options, WriteBatch};
use std::{path::Path, sync::Arc};

const META_NEXT_HEIGHT: &[u8] = b"meta/next_height";
const META_LAST_INDEXED_HEIGHT: &[u8] = b"meta/last_indexed_height";
const META_MIN_HEIGHT: &[u8] = b"meta/min_height";
const HEIGHT_PREFIX: &[u8] = b"height/";
const CANDLE_PREFIX: &[u8] = b"candle/";

#[derive(Clone)]
pub struct PriceStore {
    db: Arc<DB>,
}

impl PriceStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create db parent {}", parent.display()))?;
        }

        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path)
            .with_context(|| format!("failed to open RocksDB at {}", path.display()))?;
        Ok(Self { db: Arc::new(db) })
    }

    pub fn initialize_cursor(&self, min_height: u64) -> Result<u64> {
        if let Some(next_height) = self.read_u64(META_NEXT_HEIGHT)? {
            return Ok(next_height);
        }

        // Store the next height to index, rather than blindly storing "last
        // indexed", so the initial min_height is indexed exactly once and crash
        // recovery never has to guess whether min_height was already written.
        let mut batch = WriteBatch::default();
        batch.put(META_NEXT_HEIGHT, min_height.to_be_bytes());
        batch.put(META_MIN_HEIGHT, min_height.to_be_bytes());
        self.db
            .write(batch)
            .context("failed to initialize db cursor")?;
        Ok(min_height)
    }

    pub fn next_height(&self) -> Result<u64> {
        self.read_u64(META_NEXT_HEIGHT)?
            .ok_or_else(|| anyhow!("db cursor is missing; initialize_cursor must run first"))
    }

    pub fn put_price_point(&self, point: &PricePoint) -> Result<()> {
        let next_height = point
            .height
            .checked_add(1)
            .ok_or_else(|| anyhow!("height overflow"))?;
        let encoded = serde_json::to_vec(point).context("failed to encode price point")?;

        let mut batch = WriteBatch::default();
        batch.put(height_key(point.height), encoded);
        batch.put(META_LAST_INDEXED_HEIGHT, point.height.to_be_bytes());
        batch.put(META_NEXT_HEIGHT, next_height.to_be_bytes());
        self.db.write(batch).context("failed to write price point")
    }

    pub fn get_price_point(&self, height: u64) -> Result<Option<PricePoint>> {
        self.db
            .get(height_key(height))
            .context("failed to read price point")?
            .map(|raw| serde_json::from_slice(&raw).context("failed to decode price point"))
            .transpose()
    }

    pub fn latest_price_point(&self) -> Result<Option<PricePoint>> {
        let Some(height) = self.read_u64(META_LAST_INDEXED_HEIGHT)? else {
            return Ok(None);
        };
        self.get_price_point(height)
    }

    pub fn put_candles(&self, candles: &[CachedCandle]) -> Result<usize> {
        if candles.is_empty() {
            return Ok(0);
        }

        let mut batch = WriteBatch::default();
        for candle in candles {
            let encoded = serde_json::to_vec(candle).context("failed to encode cached candle")?;
            batch.put(candle_key(candle.candle_open_time), encoded);
        }
        self.db
            .write(batch)
            .context("failed to write candle batch")?;
        Ok(candles.len())
    }

    pub fn last_candle_open_time(&self) -> Result<Option<u64>> {
        let seek_key = candle_key(u64::MAX);
        for item in self
            .db
            .iterator(IteratorMode::From(&seek_key, Direction::Reverse))
        {
            let (key, _) = item.context("failed to iterate candles")?;
            if !key.starts_with(CANDLE_PREFIX) {
                return Ok(None);
            }
            return Ok(Some(candle_open_time_from_key(&key)?));
        }
        Ok(None)
    }

    pub fn find_candle_for_timestamp(&self, timestamp: u64) -> Result<Option<CachedCandle>> {
        let seek_key = candle_key(timestamp - (timestamp % crate::binance::FIFTEEN_MINUTES_SECS));
        for item in self
            .db
            .iterator(IteratorMode::From(&seek_key, Direction::Reverse))
        {
            let (key, value) = item.context("failed to iterate candles")?;
            if !key.starts_with(CANDLE_PREFIX) {
                return Ok(None);
            }
            let candle_open_time = candle_open_time_from_key(&key)?;
            let candle_close_time = candle_open_time
                .checked_add(crate::binance::FIFTEEN_MINUTES_SECS)
                .ok_or_else(|| anyhow!("candle close timestamp overflow"))?;
            if timestamp < candle_open_time || timestamp >= candle_close_time {
                return Ok(None);
            }
            return serde_json::from_slice(&value)
                .context("failed to decode cached candle")
                .map(Some);
        }
        Ok(None)
    }

    fn read_u64(&self, key: &[u8]) -> Result<Option<u64>> {
        self.db
            .get(key)
            .context("failed to read metadata value")?
            .map(|raw| {
                let bytes: [u8; 8] = raw
                    .as_slice()
                    .try_into()
                    .map_err(|_| anyhow!("metadata value was not a u64"))?;
                Ok(u64::from_be_bytes(bytes))
            })
            .transpose()
    }
}

fn height_key(height: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(HEIGHT_PREFIX.len() + 8);
    key.extend_from_slice(HEIGHT_PREFIX);
    key.extend_from_slice(&height.to_be_bytes());
    key
}

fn candle_key(candle_open_time: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(CANDLE_PREFIX.len() + 8);
    key.extend_from_slice(CANDLE_PREFIX);
    key.extend_from_slice(&candle_open_time.to_be_bytes());
    key
}

fn candle_open_time_from_key(key: &[u8]) -> Result<u64> {
    let raw = key
        .strip_prefix(CANDLE_PREFIX)
        .ok_or_else(|| anyhow!("not a candle key"))?;
    let bytes: [u8; 8] = raw
        .try_into()
        .map_err(|_| anyhow!("invalid candle key length"))?;
    Ok(u64::from_be_bytes(bytes))
}
