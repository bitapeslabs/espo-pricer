#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

cargo run -- --config config.json
