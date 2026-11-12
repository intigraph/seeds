# Bounded full-field search

The [terminal recording](bounded-search.typescript) runs the same
`sd search needle --full --all --limit 25 --json` against an isolated synthetic
fixture of 3,000 Actions with a complete substring index. Before the change,
the proxy refuses request 21 after forwarding 20; no partial result is accepted.
After the change, the exact 25-item page and total of 3,000 arrive in seven
requests. Positive and zero-result controls pass on both versions.

The recorded base is `ff1fa8e60260dedae857bbd31a0e472bca01aff4`; the candidate
implementation is `3ca1553a8f29990c317b0e06a6d7076be0c66f25`. The subsequent
release-only merge preserves the demonstrated search code. Both runs use
Quipu 0.17.0 (`f3982337`). This demonstrates candidate request cost and result
parity on synthetic data; it does not establish production memory usage or
installed acceptance.

Set `BEFORE_SD`, `AFTER_SD`, `QUIPU_CLI`, and `QUIPU_SERVER_BINARY` to absolute
paths to the corresponding executables, then run:

```sh
uv run --no-project docs/pr-demos/bounded-search.py
```

Download the [output](bounded-search.typescript) and
[timing](bounded-search.timing) files and replay with util-linux:

```sh
scriptreplay --log-timing docs/pr-demos/bounded-search.timing \
  --log-out docs/pr-demos/bounded-search.typescript
```

The output file also serves as a readable transcript. These small, sanitized
recording artifacts are retained with the source in this demo directory.
