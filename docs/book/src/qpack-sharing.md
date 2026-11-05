# Sharing a ledger as a pendant

A project is one named graph, and that graph is the ledger. Seeds exports it
as a Quipu **pendant**, a directory of share files, and imports another pendant
into the configured store.

## Export and import

```bash
sd export --to ledger-pendant
sd import ledger-pendant
```

The export carries `export.nt` (the ledger), `shapes.ttl` (its constraints),
`manifest.json` and `manifest.ttl` (producer, transaction and hashes), and
`.gitattributes` for Git merging. Seeds exports data and shapes; its readiness
logic remains in seeds. Quipu can separately carry registered stored queries
in `queries.ttl`; that does not make seeds' ready query a registered query.

A seeds pendant is marked `destination: internal`: no outward identifier
scrub ran. Export is not permission to publish private comments or internal
names. Review what leaves before sharing it outside its original scope.

## Reconcile a repository ledger

Configure `[pendant] dir` to keep the ledger beside the code. Every command
reconciles the working store with that directory. When both changed since
last matching, seeds refuses to choose a side silently. An explicit
`sd import <dir> --prefer pendant|store` resolves that choice;
`--replace` instead makes the store match the pendant, removing absent seeds.
See [Storage modes](storage-modes.md) for the configuration, manifest handling
and the three-way merge rules.

The local ledger can retain its history while the exported pendant contains
current facts. A remote export acquires unpinned current facts: it refuses
`export --at`, and the manifest transaction belongs to the local scratch
producer, not a historical remote snapshot. Do not use it as a cross-team pin.
Historical work-item reads such as `show --at` are a separate capability.


## Repository publisher embedding provider


A new repository build requires these trusted-host inputs, with no ambient
configuration or model-cache fallback:

| Input | Meaning |
|---|---|
| `QUIPU_SHARE_MODEL_PATH` | Absolute path to the admitted ONNX model |
| `QUIPU_SHARE_MODEL_SHA256` | Lowercase SHA-256 of those exact bytes |
| `QUIPU_SHARE_TOKENIZER_PATH` | Absolute path to the admitted tokenizer |
| `QUIPU_SHARE_TOKENIZER_SHA256` | Lowercase SHA-256 of those exact bytes |
| `QUIPU_SHARE_EMBEDDING_DIMENSION` | Explicit integer output width, 1–4096 |

The existing engine source, CLI, companion server, and indexer pins remain
required by the publisher. The builder snapshots the two bounded regular files
into its private directory, verifies the bytes while copying, and configures
Quipu there with `[quipu.embedding]`. It does not configure the host or rely on
Bobbin's separate index-model setting. The exported `manifest.embedding` must
match the exact model and tokenizer digests and dimension. A missing or
different recipe fails before receiver admission or publication.

This source contract does not enable any producer or select model bytes.
Existing tagged releases and their artifacts remain unchanged. Adoption still
requires a qualified engine/provider combination, its native output-width and
receiver controls, and the managed publication route. Synthetic byte/config
tests prove the plumbing and refusal boundaries, not ONNX compatibility or
semantic-search readiness. The existing publication test entrypoint runs them
without loading a model or calling a production service.
