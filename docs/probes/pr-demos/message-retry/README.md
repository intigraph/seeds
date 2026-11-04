# A keyed retry across a week boundary

One delivery key is sent at `2026-11-01T23:59:59Z`, then retried at
`2026-11-02T00:00:01Z`. Watch the second `written` result and stored count:
baseline writes **2 messages**; candidate writes **1**, retaining the original
Sunday send time. Both leave the board empty.

This is a real in-memory library demonstration, using the same
[example](../../../../examples/message_retry.rs) on baseline
`8891be3604728d7e1b8ce56308c783799f99a1c4` and this proposal. The exact candidate
source bytes are in [source-sha256.json](source-sha256.json). The PR links the
recording at the published candidate commit. It proves source behavior, not a
production rollout; separate tests exercise the actual shared-server applier,
concurrent writers and lost-response recovery.

The [readable transcript](output.txt) normalizes terminal line endings. Download
the [lossless output recording](output.txt.gz) with [timing.txt](timing.txt), then
replay the original bytes with:

```sh
gzip -dc output.txt.gz > output.raw
scriptreplay --log-out output.raw --log-timing timing.txt
```

Replay was checked locally. This is a terminal recording, not an inline video.
The small files are retained in the repository's existing demo directory.

To repeat, copy `examples/message_retry.rs` into a baseline checkout. Set
`BASE_DIR`, `CANDIDATE_DIR` and a **separate** `BASE_TARGET_DIR`, then run
`bash run-demo.sh`. Separate target directories prevent an artifact built from
one source tree being reused for the other. Both executions use disposable
in-memory stores; no credentials, service or installed configuration is needed.
