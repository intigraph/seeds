#!/usr/bin/env bash
# arming: ci — one synthetic owned producer, with pinned inputs before execution.
set -euo pipefail
cd "$(dirname "$0")/.."
producer=$(cargo build --locked --manifest-path tests/fixtures/acquisition-producer/Cargo.toml --message-format=json | python3 -c 'import json,sys; print(next(x["executable"] for x in list(map(json.loads,sys.stdin)) if x.get("executable") and x["target"]["name"]=="seeds-acquisition-fixture"))')
# Compile first so the exact tested SDK executable is also identified and hashed.
sdk=$(cargo test --locked --lib bounded_acquisition_g2_g3_then_g4_g6 --no-run --message-format=json | python3 -c 'import json,sys; print(next(x["executable"] for x in list(map(json.loads,sys.stdin)) if x.get("executable") and x["target"]["name"]=="seeds"))')
git rev-parse HEAD
sha256sum Cargo.toml Cargo.lock src/native/remote.rs src/native/remote/acquisition_fixture.rs \
  tests/fixtures/acquisition-producer/{Cargo.toml,Cargo.lock,src/main.rs} "$producer" "$sdk"
export SEEDS_ACQUISITION_FIXTURE_DRIVER="$producer"
"$sdk" --exact native::remote::acquisition_fixture::bounded_acquisition_g2_g3_then_g4_g6 --ignored --nocapture
