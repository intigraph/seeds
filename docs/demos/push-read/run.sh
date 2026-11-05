#!/bin/sh
# Only an isolated fixture; never starts a live consumer or contacts a service.
set -eu
: "${SEEDS_PUSH_DEMO_TEST_BINARY:?Set the compiled Seeds library test binary}"
printf '%s\n' 'Isolated SDK producer/applier + private IPC, same CLI read before/after push.'
"$SEEDS_PUSH_DEMO_TEST_BINARY" --exact native::read_stream::tests::demo_cli_push_projection_without_primary_queries --nocapture
