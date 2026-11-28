# Opt-in graph-set client

The native Unix client has a source pilot for an explicitly routed owner.
It is disabled unless `SEEDS_GRAPH_SET_PROFILE` names a private, user-owned
JSON file. Without that selection the existing requests and output stay the
same. Selecting a profile requires a remote primary, one named bearer and
already provisioned project, ephemeral and provenance graphs. Signing is
refused rather than downgraded.

The profile has these fields:

```json
{
  "base": "https://graph.example.test",
  "project": "urn:example:project",
  "store_id": "expected-owner-store-id",
  "log_epoch": "expected-owner-log-epoch",
  "principal": "declared-expected-principal",
  "credential_id": "declared-expected-credential-id",
  "denied_control_graph": "urn:example:ungranted-control",
  "state_dir": "/private/user/state/seeds/attempts"
}
```

`base` and `project` must equal the selected remote configuration. Principal
and credential ID are operator declarations, not authentication or the actor
field. The bearer authenticates each request; its digest binds recovery to the
same credential without storing the bearer. `state_dir` defaults to
`$XDG_STATE_HOME/seeds/attempts`, or `$HOME/.local/state/seeds/attempts`.
It must be absolute and private. The profile and attempts must have no group
or other permissions and must be regular, singly linked files owned by the
current user. Symlinks are refused. Concurrent invocations share a nonblocking
journal lock; the second invocation refuses before sending a write.

Each invocation first requires `/version.routed_update_completion` to advertise
an enabled `/update` v2 completion contract and the expected status and seal
routes. It then verifies a granted composed `ASK` and an ungranted-graph control
that must return HTTP 403 with only `composed read denied`. Missing support,
wrong lineage, incomplete answers and denied reads refuse without a legacy
fallback. Reads use `/graph-sets/query` with all three explicit graphs and
validate the returned composition cut. Historical pins are unsupported; a
behind-witness cut is not a historical replay facility.

The initial command set is create, update, close, comments add/list, show,
list, ready, blocked, search and count. Other commands refuse. Bulk create,
stdin payloads and file-indirected agent context are outside this pilot.
Description and comment files are bounded to 1 MiB and included by content in
the intent digest; a changed file refuses before send. The global semantic
resolver and advisory directive hints are not queried in this mode; creation
uses the existing active-item intake corpus on the admitted owner view.
Projection routes are not attempted; owner reads use explicit SPARQL.

Before sending, the client publishes one 0600 attempt envelope using a fresh
staging file, file fsync, atomic publication and directory fsync. It retains the
exact update text and form, generated write ID, body and intent digests, item
IDs, expected state and provenance, owner lineage and credential binding.
This local custody record is not a ledger or an authorization grant.

A send occurs once. Recovery calls authenticated `/update/status`, never
replays the old update, and seals an admitted uncommitted attempt through
`/update/seal`. Unknown, stopped, malformed or unavailable custody remains on
disk. Elapsed time does not authorize another write. Only a verified finished
or sealed attempt can be removed.

A complete receipt must bind the expected store/epoch, witness transaction
and exact selected graph cut. A commit witness alone returns exit 8 with state
`committed_witness_only`, its write ID, owner and local transaction. It can
be promoted only when one authenticated composed answer proves all intended
state and the exact operation provenance at or beyond the witness. Such a
`committed_proven` result retains custody until the server finishes the
receipt or a later authorized recovery resolves it.

Before dispatch, the client resolves pending attempts. The intent digest
includes the parsed command, normalized declared arguments, actor and project;
it excludes clocks, generated IDs and display-only create `--silent`.
A matching intent returns exit 0 with a typed recovered record and original
item IDs instead of dispatching again. A sealed record says `applied:false`;
resolution does not claim that the old intent ran. A different intent reports
`recovered_prior` and dispatches normally with a new ID. Unresolved custody
blocks the same intent or item; a different item may proceed with a warning.

Opt-in mutation JSON wraps ordinary output in `data` and includes a typed
`completion` record. Legacy `tx` slots are null: the owner's `local_tx` is
available only under the owner receipt and must never be treated as a main
store transaction. Recovery of a prior different intent adds
`recovered_prior`; human output carries the typed receipt on stderr. Disabled
mode retains its existing JSON contract.

Source qualification does not activate routing or authorize a cutover. Live
credential, owner provisioning, composed cross-owner joins, CI, review and
installed acceptance remain separate gates. Turn off profile selection to
restore the legacy client path; retain unresolved envelopes for later recovery.
