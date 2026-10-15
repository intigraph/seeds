# Combined remote preflight

Remote commands refuse ledgers containing the retired `seeds:revision`
predicate. The shared and ephemeral graph checks now share one bounded query
with the ephemeral presence probe. Each positive branch emits at most one tag;
a completion sentinel distinguishes a complete negative answer from missing
bindings. Unknown, duplicate, wrongly typed or truncated results refuse.

Only positive ephemeral presence is remembered, as before. An empty answer
never survives into another operation: a later read still checks presence and
can observe another writer's new work. Current seed revision checks and atomic
write preconditions are unchanged.

On an empty ephemeral graph, the two vocabulary requests become one; the later
presence request remains. On a populated graph, the preflight also establishes
positive presence, avoiding the later existence request. This is a transport
call reduction, not a claim about a production request-rate target.

`combined_preflight_native_scope_counts_and_new_facts` runs against a disposable
Quipu server: one preflight request, new ephemeral facts after an empty check,
three requests for preflight plus both populated fact reads, rejection of old
vocabulary in either graph, and isolation from a foreign graph. Reverting the
preflight to the previous implementation fails the request-count control
(two versus one). Parser controls reject incomplete or ambiguous responses.

## Demo

[Terminal recording](assets/combined-preflight-output.txt) and
[timing](assets/combined-preflight-timing.txt) show actual private HTTP requests
to the CI-pinned Quipu 0.11 binary. Before: two vocabulary queries return a
clean ledger. After: the query extracted from `remote.rs` returns the same
answer in one request. A later insertion is observed by a fresh preflight.
This demonstrates the wire contract; the Rust native tests separately establish
cache behavior and refusals. It is not production traffic or installed proof.

Run `SEEDS_TEST_QUIPU_SERVER=/path/to/quipu-server python3
 docs/design/assets/combined-preflight-demo.py`, or replay the downloaded text
files with `scriptreplay --log-out combined-preflight-output.txt --log-timing
combined-preflight-timing.txt`. The recording includes the source SHA256.
