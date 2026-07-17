# espopricer

`espopricer` indexes BTC/USD prices by Bitcoin block height.

On startup it opens its RocksDB database, initializes the next height cursor to `min_height` when the database is empty, asks Metashrew for the indexed tip with `metashrew_height`, reads the timestamp for the last indexed height anchor, and batch-fills every Binance 15 minute candle from that point through the latest candle. Only after the candle cache is caught up does it index missing block heights through the Metashrew tip.

For each height it reads the Bitcoin Core block header timestamp, binary-searches the locally cached candle keys for the matching 15 minute candle, and stores an O(1) RocksDB lookup of:

```text
height -> { timestamp, candle_open_time, price_scaled, price_raw }
```

After catchup, it keeps polling Metashrew for a higher tip. When a new height appears, it first catches up any missing Binance candles through the latest candle, then indexes the new heights from the local candle cache.

## Config

Create `config.json` from `config.json.sample`:

```json
{
  "rpc_host": "127.0.0.1",
  "rpc_port": 8080,
  "binance_api_key": "",
  "binance_api_secret": "",
  "binance_base_url": "https://data-api.binance.vision",
  "binance_symbol": "BTCUSDT",
  "metashrew_rpc_url": "http://127.0.0.1:8081",
  "db_path": "./db/espopricer.rocks",
  "min_height": 0,
  "bitcoin_core_rpc_host": "127.0.0.1",
  "bitcoin_core_rpc_port": 8332,
  "bitcoin_core_rpc_username": "bitcoinrpc",
  "bitcoin_core_rpc_password": "change-me",
  "poll_interval_ms": 5000,
  "price_decimals": 8
}
```

`bitcoin_core_rpc_url` can be supplied instead of `bitcoin_core_rpc_host` and `bitcoin_core_rpc_port`.

`price_scaled` is the Binance close string with decimals. `price_raw` is that same price multiplied by `10^price_decimals` and returned as a base-10 integer string.

The default Binance host is `https://data-api.binance.vision` because this service only needs public market data. `https://api.binance.com` can return HTTP 451 in restricted regions.

Use a `min_height` whose timestamps are covered by Binance `BTCUSDT` candles.

## Run

```bash
cargo run -- --config config.json
```

Set `RUST_LOG=debug` for more indexing logs.

## RPC

The service exposes one JSON-RPC method at `/`:

```bash
curl -sS http://127.0.0.1:8080/ \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"get_btc_price_at_height","params":{"height":850000}}'
```

If `params` is omitted, `null`, `{}`, `[]`, or `"latest"`, the latest indexed height is returned:

```bash
curl -sS http://127.0.0.1:8080/ \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"get_btc_price_at_height"}'
```

Response:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "height": 850000,
    "price_scaled": "64123.45000000",
    "price_raw": "6412345000000"
  },
  "id": 1
}
```
