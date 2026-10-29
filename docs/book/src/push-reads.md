# Push-backed local reads

On Unix, `sd read-stream --config /absolute/private/reader.json` runs one
resident consumer of the existing Quipu replication applier. Short-lived
`list`, `ready`, `blocked`, and `show` commands can read its private Unix socket.
The existing engine renders each result at one local transaction; source and
local transaction identifiers remain distinct.

This is an opt-in transport, not a new tracker. Writes and claims retain the
configured primary backend. Explicit historical `--at` reads retain their
existing path. Configuring this reader does not route other clients or eliminate
polling performed independently by another application.

## Admission packet

Only the user configuration may set `[quipu] read_stream_config` to an absolute
packet path. A project configuration cannot select this transport. The packet
must be a bounded regular file owned by the current UID, with no group or other
permissions. Its JSON fields are:

| Field | Meaning |
|---|---|
| `primary_url`, `graph` | Exact existing authoritative backend URL and board graph |
| `principal` | Operator-admitted registry principal; not a response attestation |
| `replication_credential_file` | Absolute path to a separate private Replicate-only bearer file |
| `store_id`, `log_epoch` | Admitted source identity and epoch |
| `scope_digest`, `policy_digest` | Admitted shared-applier scope and policy |
| `stream_key` | 64 hexadecimal characters encoding the admitted public signing key |
| `manifest` | Absolute path to the private bootstrap manifest |
| `manifest_sha256` | SHA-256 of the admitted manifest bytes |
| `bootstrap` | Absolute path to the private bootstrap SSE frames |
| `socket` | Absolute Unix socket path in a UID-owned private directory |
| `max_freshness_ms` | Required bound, from 1 through 5000 milliseconds |

These trust inputs must come from the reviewed upstream admission process.
They are not learned from an untrusted response. The manifest must cover the
entire board and its ephemeral graph, without additional context graphs. The
existing applier verifies bootstrap completion, signatures, scope and epoch.
No automatic bootstrap cut, credential registration, or service installation is
performed by this command.

Run `sd read-stream --config /absolute/private/reader.json --check` first. It
validates the pinned local inputs without opening a socket or network connection.
This check does not prove live authorization or freshness.

## Read-only projection queries

`sd read-stream-query` accepts a JSON object with one `query` string on stdin
and requires the same user-level reader packet. It uses the same private socket
and proof validation. It does not open another backend. Only SELECT and ASK
queries are admitted, with parsed graph patterns restricted to constant names
of the board and ephemeral graphs. ROOT reads, variable graph names, external
services, dataset/BASE overrides, updates and unsupported algebra refuse. Scope
checks descend into EXISTS, filters, ordering and aggregates. Query time and
intermediate rows are bounded; a budget failure refuses rather than returning
a truncated empty answer. JSON results retain full term values.

## Live behavior and recovery

The consumer uses only its separate Replicate-only bearer file for fixed
replication endpoints. It rereads this private, bounded regular file on every
operation; removal, rotation or malformed content never reuses cached bytes.
Ordinary read/write tokens and signing keys are never consulted. The server
registry authenticates the request principal; the packet principal is an
operator admission input, not a client response attestation. Source signing
key, epoch, scope and policy pins are separately required. Signed
challenges establish a freshness bound; a heartbeat alone cannot establish it.
The consumer issues at most one challenge every two seconds. This control
traffic is distinct from board queries or watermark polling.

A missing, stale, disconnected, mismatched or corrupt projection is reported as
`UNKNOWN`, never as an empty board. The local client charges IPC elapsed time
against the proof budget. Configured head reads do not silently fall back to
primary polling. Other board bindings refuse rather than reuse this projection.

The socket is create-only. The consumer never removes or adopts a pre-existing
socket, and retains its socket after termination. Recovery must stop the old
consumer and deliberately retire its socket through the normal supervised
operator procedure before starting a replacement. A stream failure invalidates
freshness; it does not start an automatic reconnect loop. Removing the opt-in
user setting through the normal configuration procedure restores ordinary
primary reads. Keep installation, authorization and measured traffic acceptance
separate from source and local fixture checks.
