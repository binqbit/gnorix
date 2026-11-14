#!/bin/sh
# Build and exercise the complete example in a local NEAR sandbox.
set -eu

example_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
cd "$example_dir/../.."

for manifest in \
    examples/near/Cargo.toml \
    examples/near/token/Cargo.toml \
    examples/near/tests/contracts/ft-receiver/Cargo.toml
do
    cargo near build non-reproducible-wasm --locked --no-abi \
        --manifest-path "$manifest" --out-dir target/near
done

cargo test -p gnorix-near-wallet --features sandbox --locked --tests \
    -- --ignored --nocapture
