# sd history

Every version of one seed, with the transaction that wrote it.

```bash
sd history sd-a3f
sd history sd-a3f --json
sd show sd-a3f --at 12       # read any one version in full
```

> **Not br's `history`.** br's verb manages local backup files. In seeds
> every write is a quipu transaction, so a seed's history is already in the
> store; `sd history` lists it, and says so in its output.

- One entry per version, oldest first: the transaction, the revision, when,
  the status, and **what changed** from the previous version.
- Each entry is exactly what `sd show <id> --at <tx>` reads.
- Works against a local store and a quipu server alike. A quipu server
  reports no head transaction and its writes report none either
  (aegis-xajsgn), so history finds each version by searching `--at` reads,
  a handful of reads per version. Each remote read binds the exact item in
  both project graphs; unrelated items and comments are never hydrated.
  A historical read refused by the server (including attached-store transaction
  ambiguity) remains a refusal, with no current-state or whole-board fallback.
- A detected revision gap or terminal version differing from the current item
  refuses the result instead of returning a partial version list. This can
  occur with deletion/recreation or imported revision sequences.
- `--at` does not apply: history is every version.

**`--json`**: `{meaning, id, versions: [<seed object> + tx + changes]}`.

## Explicit retained-fact SDK read

`RemoteBackend::retained_history` is a separate, opt-in library API. Supply a
`HistoryRequest` containing the trusted MAIN `source_store_id`, exact subject
and project (or ephemeral) graph, row/byte caps, and an observed generation for
explicit transaction bounds. It first requires the server's
`quipu-entity-history/main-v1` contract, then validates the source receipt and
qualified transactions. Refusals and incomplete or malformed responses never
fall back to ROOT, SPARQL, current state, or transaction bisection.

`RetainedHistory::source_bytes()` preserves the original response bytes,
including the source receipt, typed values, authenticated evidence and separate
caller declarations. Its completeness covers only retained physical MAIN facts
in that subject/graph/range. It does not prove continuous seed versions, include
comment children automatically, authenticate a physical host, or make a local
transaction ID a cross-store pin. This API does not change the CLI behavior above.


## Explicit retained-facts CLI mode

For a qualified retained-fact receipt rather than a version list:

```bash
sd history sd-a3f --retained-facts --source-store-id <trusted-store-id> --json
sd history sd-a3f --retained-facts --source-store-id <trusted-store-id> --comment 1 --json
```

Obtain the source ID from trusted operator configuration. This mode never enrolls
an ID from the response or an unpinned connection audience. It requires a remote
backend, an explicitly configured named graph and JSON output, refuses `--at`, and caps the SDK acquisition at 10,000
facts, 4 MiB and 30 seconds after connection setup. An unsupported source contract
or failed receipt refuses with no alternate history path. An empty exact-subject
receipt also refuses: it does not establish that the item exists or has a readable history.

The output names `mode: retained-main-facts`, includes the validated
`source_response`, and preserves its exact UTF-8 bytes in `source_response_utf8`.
Keep the source's `continuous_history` and `historical_policy_proof` values:
a complete retained range does not establish every seed version. `--comment N`
selects one known comment subject; it does not discover all children or prove
historical comment-family closure. Authenticated identity and caller declarations
remain separate source fields; neither is relabelled as authenticated physical host.
