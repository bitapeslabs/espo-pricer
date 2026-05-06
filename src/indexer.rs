use crate::{
    binance::{
        BinanceClient, FIFTEEN_MINUTES_SECS, MAX_KLINES_PER_REQUEST, latest_candle_open_time,
        normalize_to_15m,
    },
    bitcoin_core::BitcoinCoreClient,
    metashrew::MetashrewClient,
    store::PriceStore,
    types::{CachedCandle, PricePoint},
};
use anyhow::{Context, Result, anyhow, bail};
use std::{sync::Arc, time::Duration};
use tracing::{debug, error, info, warn};

pub struct Indexer {
    store: Arc<PriceStore>,
    metashrew: MetashrewClient,
    bitcoin_core: BitcoinCoreClient,
    binance: BinanceClient,
    min_height: u64,
    poll_interval: Duration,
}

impl Indexer {
    pub fn new(
        store: Arc<PriceStore>,
        metashrew: MetashrewClient,
        bitcoin_core: BitcoinCoreClient,
        binance: BinanceClient,
        min_height: u64,
        poll_interval: Duration,
    ) -> Self {
        Self {
            store,
            metashrew,
            bitcoin_core,
            binance,
            min_height,
            poll_interval,
        }
    }

    pub async fn run(self) -> Result<()> {
        let initial_next = self.store.initialize_cursor(self.min_height)?;
        info!(
            next_height = initial_next,
            min_height = self.min_height,
            "indexer cursor ready"
        );

        loop {
            if let Err(err) = self.sync_once().await {
                error!(error = ?err, "indexer sync failed; retrying after poll interval");
            }
            debug!(
                poll_interval_ms = self.poll_interval.as_millis(),
                "sleeping before next metashrew poll"
            );
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    async fn sync_once(&self) -> Result<()> {
        let tip = self
            .metashrew
            .tip_height()
            .await
            .context("failed to read metashrew tip")?;
        let next_height = self.store.next_height()?;

        info!(next_height, metashrew_tip = tip, "metashrew poll complete");

        if next_height > tip {
            debug!(
                next_height,
                metashrew_tip = tip,
                "already caught up to metashrew tip"
            );
            return Ok(());
        }

        self.catch_up_candles_from_anchor_height(next_height)
            .await?;
        self.index_heights(next_height, tip).await?;
        Ok(())
    }

    async fn catch_up_candles_from_anchor_height(&self, next_height: u64) -> Result<()> {
        let anchor_height = if next_height > self.min_height {
            next_height - 1
        } else {
            next_height
        };

        info!(
            anchor_height,
            next_height, "reading block timestamp for candle backfill anchor"
        );
        let anchor_timestamp = self
            .bitcoin_core
            .block_timestamp(anchor_height)
            .await
            .with_context(|| {
                format!("failed to read timestamp for candle anchor height {anchor_height}")
            })?;
        let start_open_time = normalize_to_15m(anchor_timestamp);
        info!(
            anchor_height,
            anchor_timestamp, start_open_time, "candle backfill anchor located"
        );

        self.catch_up_candles(start_open_time).await
    }

    async fn catch_up_candles(&self, requested_start_open_time: u64) -> Result<()> {
        let latest_open_time = latest_candle_open_time()?;
        if requested_start_open_time > latest_open_time {
            bail!(
                "requested start candle {} is newer than latest local 15m candle {}; retrying later",
                requested_start_open_time,
                latest_open_time
            );
        }
        let last_cached_open_time = self.store.last_candle_open_time()?;
        let mut cursor = match last_cached_open_time {
            Some(last) if last >= requested_start_open_time => last
                .checked_add(FIFTEEN_MINUTES_SECS)
                .ok_or_else(|| anyhow!("candle cursor overflow"))?,
            _ => requested_start_open_time,
        };

        // The newest kline can still be active. Refresh it on each live catchup
        // pass instead of letting an early partial close stay cached forever.
        if cursor > latest_open_time {
            cursor = latest_open_time;
        }

        info!(
            requested_start_open_time,
            ?last_cached_open_time,
            latest_open_time,
            fetch_start_open_time = cursor,
            "starting Binance 15m candle catchup"
        );

        while cursor <= latest_open_time {
            let max_batch_span = u64::from(MAX_KLINES_PER_REQUEST.saturating_sub(1))
                .checked_mul(FIFTEEN_MINUTES_SECS)
                .ok_or_else(|| anyhow!("candle batch span overflow"))?;
            let batch_end = cursor
                .checked_add(max_batch_span)
                .map(|candidate| candidate.min(latest_open_time))
                .ok_or_else(|| anyhow!("candle batch end overflow"))?;

            info!(
                batch_start_open_time = cursor,
                batch_end_open_time = batch_end,
                "requesting Binance candle batch"
            );
            let candles = self.binance.candles(cursor, batch_end).await?;
            if candles.is_empty() {
                bail!(
                    "Binance returned no candles for requested range {}..={}",
                    cursor,
                    batch_end
                );
            }

            validate_candle_batch(cursor, &candles)?;
            let last_batch_open_time = candles
                .last()
                .map(|candle| candle.candle_open_time)
                .ok_or_else(|| anyhow!("validated candle batch was unexpectedly empty"))?;
            let stored = self.store.put_candles(&candles)?;
            info!(
                stored,
                first_open_time = candles[0].candle_open_time,
                last_open_time = last_batch_open_time,
                "stored Binance candle batch"
            );

            cursor = last_batch_open_time
                .checked_add(FIFTEEN_MINUTES_SECS)
                .ok_or_else(|| anyhow!("candle cursor overflow"))?;
        }

        info!(
            latest_open_time,
            last_cached_open_time = ?self.store.last_candle_open_time()?,
            "Binance candle catchup complete"
        );
        Ok(())
    }

    async fn index_heights(&self, start_height: u64, tip: u64) -> Result<()> {
        info!(
            from = start_height,
            to = tip,
            total = tip - start_height + 1,
            "indexing BTC/USD prices from cached candles"
        );

        let mut height = start_height;
        while height <= tip {
            self.index_height_from_cached_candles(height).await?;
            if height % 1_000 == 0 || height == tip {
                info!(height, tip, "height price indexing progress");
            }
            height += 1;
        }

        info!(tip, "height indexing reached metashrew tip");
        Ok(())
    }

    async fn index_height_from_cached_candles(&self, height: u64) -> Result<()> {
        // Metashrew owns the indexing tip, but Bitcoin Core is still the
        // timestamp authority here because this service only needs raw header
        // time and Metashrew's public RPC does not expose block headers.
        let timestamp = self
            .bitcoin_core
            .block_timestamp(height)
            .await
            .with_context(|| format!("failed to read block timestamp at height {height}"))?;

        debug!(
            height,
            timestamp, "binary-searching cached candles for block timestamp"
        );
        let candle = self
            .store
            .find_candle_for_timestamp(timestamp)?
            .ok_or_else(|| {
                anyhow!("cached 15m candle missing for height {height}, timestamp {timestamp}")
            })?;

        self.put_price_point(height, timestamp, candle)?;
        Ok(())
    }

    fn put_price_point(&self, height: u64, timestamp: u64, candle: CachedCandle) -> Result<()> {
        debug!(
            height,
            timestamp,
            candle_open_time = candle.candle_open_time,
            price_scaled = %candle.price_scaled,
            price_raw = %candle.price_raw,
            "writing height price point"
        );
        let point = PricePoint {
            height,
            timestamp,
            candle_open_time: candle.candle_open_time,
            price_scaled: candle.price_scaled,
            price_raw: candle.price_raw,
        };
        self.store.put_price_point(&point)?;
        Ok(())
    }
}

fn validate_candle_batch(expected_start_open_time: u64, candles: &[CachedCandle]) -> Result<()> {
    let mut expected = expected_start_open_time;
    for candle in candles {
        if candle.candle_open_time != expected {
            warn!(
                expected_open_time = expected,
                actual_open_time = candle.candle_open_time,
                "Binance candle batch has a gap or shifted start"
            );
            bail!(
                "Binance candle batch gap: expected open time {}, got {}",
                expected,
                candle.candle_open_time
            );
        }
        expected = expected
            .checked_add(FIFTEEN_MINUTES_SECS)
            .ok_or_else(|| anyhow!("candle validation overflow"))?;
    }
    Ok(())
}
