#!/bin/sh
set -eu

cargo build --release --target wasm32-unknown-unknown --package ocs_web_worker
mkdir -p web/worker_pkg
wasm-bindgen \
  --target web \
  --out-dir web/worker_pkg \
  --out-name ocs_web_worker \
  target/wasm32-unknown-unknown/release/ocs_web_worker.wasm
