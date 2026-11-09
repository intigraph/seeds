#!/usr/bin/env bash
set -euo pipefail
: "${DEMO_TEST_BIN:?compiled test executable required}"
: "${DEMO_CODE_REV:?code revision required}"
printf 'Existing-target comment registration: local disposable server fixture\n'
printf 'Base: 03257c826b348907100075a82881f68e84865bbb\n'
printf 'Code revision: %s\n' "$DEMO_CODE_REV"
printf '\nBEFORE: archived actual run of this same wire reproducer on base + test only.\n'
cat docs/demos/comment-registration/before.txt
printf '\nAFTER: actual run on changed code; assertion requires zero graph-list requests.\n'
printf '$ comment-tests --exact native::remote::tests::existing_comment_write_skips_graph_discovery_and_keeps_history --nocapture\n'
"$DEMO_TEST_BIN" --exact native::remote::tests::existing_comment_write_skips_graph_discovery_and_keeps_history --nocapture
printf '\nSame target, two old comments + one append: all three texts preserved.\n'
printf 'This is a fixture proof of registry-read elimination, not installed latency acceptance.\n'
