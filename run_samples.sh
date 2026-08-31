#!/bin/bash
echo "Running samples/showcase.craw..."
cargo run --bin craw -- run "samples/showcase.craw"
if [ $? -ne 0 ]; then
    echo "FAIL: samples/showcase.craw"
    exit 1
else
    echo "SUCCESS: samples/showcase.craw"
fi
