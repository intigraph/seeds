# Local push-read fixture

The trigger is a producer title update. The same `sd --json list` dispatcher
reads `before`, then `after push` through private IPC and the shared replication
applier. Source transactions and local transactions differ. A positive-control
listener observes the configured primary destination; neither read opens a
primary connection. Disconnect produces UNKNOWN instead of HTTP fallback.

This is a source fixture, not installed acceptance, a live authorized upstream,
or a before/after measurement of fleet traffic. Proof timestamps are injected
fixture times; the displayed millisecond bound is not a latency benchmark. The old source base
`46c05b1e2f1452069abd2bdce667d92846de960c` has no private IPC command; the shared
measure here is the same CLI read before and after the pushed fixture event.
The recorded candidate is the commit containing this demo (resolve its exact
revision with `git log -1 --format=%H -- docs/demos/push-read/run.sh`).

Build the library test executable with `cargo test --locked --lib --no-run` and
set `SEEDS_PUSH_DEMO_TEST_BINARY` to the resulting executable. Run
`sh docs/demos/push-read/run.sh`. Use the deployment's normal isolated HOME test
guard. Only generated synthetic data and local fixture sockets are used.

The small output/timing recording is retained beside this caption for reviewer
replay, not as an inline video. Replay with
`gzip -dc output.gz > /tmp/push-read-replay.output` then
`scriptreplay --log-out /tmp/push-read-replay.output --log-timing replay.timing`.
`output.gz` preserves the original terminal bytes for timing; `output.txt` is
the readable transcript with CRLF normalized to LF.
