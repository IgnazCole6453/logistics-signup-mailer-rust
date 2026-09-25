#!/bin/sh
set -eu

: "${INFRAI_API_KEY:?set INFRAI_API_KEY first}"
cargo run --bin shipment-signup -- \
  "chenhua@changba.com" \
  "change-this-local-demo-password" \
  "Night Dispatch" \
  "SHP-2048" \
  "http://localhost:3000"
