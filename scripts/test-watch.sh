#!/usr/bin/env bash
# Runs cargo test in watch mode, logging results to tmp/tests.log (overwritten each run).
set -euo pipefail

cd "$(dirname "$0")/.."

mkdir -p tmp

exec cargo watch -s 'cargo test --color always 2>&1 | tee tmp/tests.log'
