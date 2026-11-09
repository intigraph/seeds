# Existing-target comment registration

Appending a comment to an item already read from the project no longer lists
all registered graphs. The same disposable-server wire reproducer observed
one graph-list request before and asserts zero afterward, while preserving
both existing comments and the appended text.

Base: `03257c826b348907100075a82881f68e84865bbb`, with the new reproducer only.
Changed code: `54966a2` (`src/native/remote.rs`). The documentation commit does
not change that code. The fixture uses a local, clean Quipu server at revision
`360f8f49ff816aa5edcbaae3fa752515e0dfcf25`.

[Terminal output](output.txt) and [timing](timing.txt) form an output-only
recording. It starts by displaying the archived actual before-failure, then
runs the changed-code reproducer live. The before run is not recreated by
narration. Replay after downloading both files:

```sh
scriptreplay --log-out output.txt --log-timing timing.txt
```

To reproduce the after control, build `cargo test --lib --no-run`, set
`SEEDS_TEST_QUIPU_SERVER` to a disposable server executable, and run the test
executable with:

```text
--exact native::remote::tests::existing_comment_write_skips_graph_discovery_and_keeps_history --nocapture
```

Creation, ephemeral routing and concurrent-comment index conflicts have
separate wire controls. This recording demonstrates registry-read elimination
on a small fixture. It does not measure production latency or prove installed
acceptance. The small recording and caption remain in this directory with
repository history; no binary executable or private incident log is published.
