# Explicit verified acquisition API

`RemoteBackend::acquire_verified` is a library-only surface. Ordinary snapshot
reads, pendant export and the CLI do not call it. In particular, remote
`export --at` still refuses before connecting or writing an artifact.

Callers supply a trusted source lineage, explicit project/optional registered
ephemeral scope, generation, requested pin and lower-only byte budget. A
consumer requiring ephemeral data must explicitly include that graph; the API
never treats missing or refused required data as empty. Comments are project
data, not a separate comments graph. Unknown graph/dataset/fork options are
not supported by the request type. Explicit pins require a generation.

Before a request, the caller binds an out-of-band `VerifierManifest` to the
consumer executable digest, SDK source and embedded lockfile digest, exact
producer source and verifier source/executable digest. Source-to-binary
provenance is an external build-manifest obligation; this API checks the
binary/lock commitments, not a signature or a claimed copyright identity.

The client recognizes only `quipu-acquisition/composed-v1` in
`/version.read_contracts`, then requests `/acquire/composed`. It refuses skew,
transport errors, byte overflow and missing completeness proof without
fallback or retry. The one-shot verifier uses the producer's authoritative
`Payload::verify` in its own dependency workspace. A DTO alone cannot admit
bytes. The client checks the verifier's protocol and exact request/wire
digests, source build, clean producer flag and canonicalization before
constructing `VerifiedAcquisition`. Hash binding is integrity verification;
it is not hostile-producer authentication or production permission.

## Artifact persistence

Only a verified result has `persist`. The artifact is one JSON file containing
the original source payload bytes (including the untouched source receipt),
the original request, and a separate local artifact receipt. There is no
substitution of a local transaction or store ID for source authority.

Staging is exclusive and private. A same-filesystem hard link publishes the
complete, fsynced file atomically without replacing an existing destination.
The parent directory is then fsynced. A failure after linking is
`Indeterminate`: read the destination before any retry. The previous artifact
is never overwritten. Cross-filesystem or unsupported hard-link operations
refuse. No store is opened or mutated during persistence.

## Bounds and unmeasured gates

The caller's wire budget is at most 256 MiB; envelope overhead counts against
it too, so admission can be stricter than the producer's graph-byte budget.
The total request timeout is at most 120 seconds, spans negotiation, streaming
and verification, and is checked before admitting a result. The separate
verifier additionally fixes Linux CPU/address-space/output limits. Persistence
has its own explicit deadline. Blocking kernel filesystem calls cannot be
preempted by these checks; no hard real-time filesystem guarantee is claimed.

The source addition has deterministic request/budget/persistence unit controls
and compilation checks. No producer acquisition, full-size trial, new server,
runtime installation, full-board read or activation is authorized by it.
Before a separately authorized trial, bind both executable hashes and source
locks, exact store/graph/generation/pin/floor, unchanged row/provider/byte/graph
caps, total deadline, isolated scope, cleanup and raw proof paths. The retained
fixture verifier is not a production adapter compatibility certificate.

Required future integration controls include current-version negotiation,
raw receipt and graph roundtrip, actual authoritative rejection of rehashed
semantic tamper, generation restart/old floor, required ephemeral absence,
transport/child deadline and interrupted artifact publication. Production
eligibility, release/install and enabling any CLI consumer remain separate.

Rollback is a normal source revert of this unwired API. No runtime or data
change is made by adding it.
