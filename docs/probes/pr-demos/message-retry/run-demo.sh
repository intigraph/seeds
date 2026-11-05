#!/usr/bin/env bash
set -euo pipefail
: "${BASE_DIR:?set BASE_DIR to baseline 8891be36 with the same example}"
: "${CANDIDATE_DIR:?set CANDIDATE_DIR to this proposal}"
echo 'Baseline 8891be3604728d7e1b8ce56308c783799f99a1c4 (in-memory library)'
echo '$ cargo run --quiet --example message_retry'
CARGO_TARGET_DIR="${BASE_TARGET_DIR:?use a separate baseline target}" cargo run --quiet --manifest-path "$BASE_DIR/Cargo.toml" --example message_retry
echo
echo 'Candidate (same input, in-memory library)'
echo '$ cargo run --quiet --example message_retry'
cargo run --quiet --manifest-path "$CANDIDATE_DIR/Cargo.toml" --example message_retry
