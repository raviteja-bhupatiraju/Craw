#!/usr/bin/env bash
set -e

# Build the compiler
cargo build --release

# Run the comprehensive feature test
echo "Running feature_test.craw..."
cargo run --release --bin craw -- run tests/feature_test.craw

# Run the master showcase
echo -e "\nRunning master_showcase.craw..."
cargo run --release --bin craw -- run samples/master_showcase.craw

echo -e "\nEverything verified!"
