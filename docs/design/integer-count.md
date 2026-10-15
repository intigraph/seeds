# Bounded comment indices across server versions

The snapshot decoder accepts comment indices from zero through `i64::MAX`.
Quipu 0.14 preserves larger `xsd:integer` terms instead of rejecting them at
parse time. Arithmetic term-identity tests alone therefore admit values that
cannot become comment indices. The indexed summary guard also checks the upper
bound, preserving the maximum valid integer and the existing type, sign and
term-identity checks. Both comment counts and full-field search use this guard;
priority summaries retain their narrower 0–255 bound.

CI tests remote behavior against checksum-pinned Quipu 0.11 and 0.14 releases.
The existing decoder-parity fixture covers valid maximum and subtype values,
oversized integers, fractional and negative values, strings, language tags,
IRIs, and project/ephemeral shadowing. An unchanged expected count of four
fails before the fix and passes afterward.

## Demo

[Output](assets/integer-count/output.txt) and
[timing](assets/integer-count/timing.txt) form a replayable terminal recording.
Replay with `scriptreplay --log-out output.txt --log-timing timing.txt`.
The [reproducer](assets/integer-count/demo.py) starts one disposable server and
queries the same synthetic numeric corpus with the old and new guards:

```sh
python3 docs/design/assets/integer-count/demo.py --server /path/to/quipu-server
```

Recorded against the official Quipu 0.14.0 server: six admitted indices become
four, while `9223372036854775807` stays present. This demonstrates the exact
query-expression delta, not an installed SDK or production measurement.
The source base is `b90e2e8a`; the fix is the explicit upper bound in
`nonnegative_integer_guard`. No production data or credentials are used.
