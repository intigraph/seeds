# Bounded comment indices across server versions

The snapshot decoder accepts comment indices from zero through `i64::MAX`.
Quipu 0.14 preserves larger `xsd:integer` terms instead of rejecting them at
parse time. Arithmetic term-identity tests alone therefore admit values that
cannot become comment indices. The indexed summary guard also checks the upper
bound, preserving the maximum valid integer and the existing type, sign and
term-identity checks. Both comment counts and full-field search use this guard;
priority summaries retain their narrower 0–255 bound.

CI tests native remote reads and the complete storage-mode suite against
checksum-pinned Quipu 0.11 and 0.14 releases. History search resolves each probe
to the next committed transaction through a bounded cursor lookup; it never
assumes a future transaction means the current state. Empty metadata bounds the
search without fabricating a snapshot; malformed metadata and lookup failures
refuse. Explicit `--at` behavior is unchanged.
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

## Committed history probe controls

`python3 tests/history_wire.py` exercises ordinary and gapped committed IDs,
a fixed complete history, malformed/oversized/truncated metadata, failed or
unexpectedly empty feeds, historical refusals, missing items and recreation.
Every history query stays scoped to the requested item; metadata lookups carry
`since` and `limit=1` and refuse responses larger than 64 KiB. Query counts,
metadata lookup counts and total response bytes stay identical for 1 and 101
unrelated board items. Explicit future `show --at` still refuses without using
the history resolver.

Before the history fix, the official 0.14 storage-mode suite passed 29 tests and
failed its history and attribution-history cases on an uncommitted transaction.
Afterward, all 31 storage-mode tests pass. The complete 0.14 CI lane is restored;
the earlier native-only intermediate was not a full compatibility acceptance.

History [output](assets/committed-history/output.txt) and
[timing](assets/committed-history/timing.txt) replay the retained pre-fix native
HTTP400 responses followed by an actual candidate CLI call against an isolated
HTTP fixture with committed IDs 1/10/20/30. It returns versions at 10/20/30.
The before segment is retained native evidence; the candidate segment uses the
wire fixture, not the same live server. Separate official-release storage-mode
tests establish the native before/after result. Replay with `scriptreplay` as
above. Source and candidate binary hashes are in the
[manifest](assets/committed-history/source-sha256.txt). This adds no new
cross-restart lineage or atomic-history guarantee beyond the existing contract.
