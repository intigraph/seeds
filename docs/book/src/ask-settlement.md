# Atomic overseer Ask settlement

The library API `ask_settlement::settle` closes an open Ask and appends its
operator ruling to the work item or epic the Ask unblocks. Both expected
revisions and the ruling comment are committed in **one Seeds `WriteBatch`**.
A concurrent change to either item refuses the entire batch. The linked work's
status, description, notes and existing comments are preserved.

The request binds the explicit project graph, Ask id/revision, linked item id/revision and exact native
attestation receipt IRI. The native `AdmittedSettlement` port must verify the
full signed proposal, declared choice, human identity, deadline, graph scope and
both item/revision bindings. A receipt for an older single-item contract cannot
supply the linked-item binding. The answer type has no JSON deserializer and no
`verified` boolean that browser input can promote into authority.

The backend must prove the same project graph; an implicit or foreign dataset
refuses. The native admission layer must also bind its backend connection to the
verified receipt store/lineage and graph-generation profile. This pure library
API selects no backend connection or production profile.

The same native port must durably reserve its one send before returning from
`reserve_send`. Its custody must survive process loss and refuse another send
for that intent. The core selects no filesystem, credential, reader or runtime
adapter. With no admitted port, settlement is disabled and refuses before a
storage read or write. This is a library transition, not a newly enabled CLI or
production service.

On success, the closed Ask and the linked ruling comment retain the exact
receipt reference. The result is returned only after both items and that comment
read back. The comment distinguishes the verified operator from the ordinary
Seeds write actor. A changed or closed linked item, changed Ask link, foreign
binding, exhausted revision or incomplete receipt fails without a sequential
fallback.

After any error or uncertain result, retain custody and call
`ask_settlement::reconcile`. This function only reads: it never reserves or sends
another write. A complete existing settlement reconciles without duplicate
comments. An absent or contradictory result remains unproven, not permission to
retry the close. Production admission must supply the trusted native receipt
reader, durable custody and scoped Seeds writer before a consumer can use this
transition; a library test does not establish that admission.

## Staged native executable port

On Unix, `sd ask-settle` supplies a concrete native port. It remains disabled
without an explicitly selected root-owned profile and `--execute` or
`--reconcile`. The disabled path does not read stdin, initialize a store or read
any credential. No deployment, service, enrollment or grant is installed by the
source change.

The bounded request contains only `graph`, `ask`, `ask_revision`, `target`,
`target_revision` and `receipt`. Unknown keys, including `verified`, refuse.
Admission never accepts a caller-supplied Answer or verification DTO. The native
process verifies hashes and ownership of the selected Node executable and seven
Rikuy modules, then supplies intent and the selected reader profile over a
private pipe. That read-only program fetches the actual native point receipt,
registered request/answer facts, complete signed proposal, graph instances and
both item bindings. Its output is private process evidence, not a public
verification API. The process has bounded input/output and an eight-second
kill/reap deadline.

The profile schema is `seeds-ask-native-port/v1`; an inert envelope is:

```json
{"schema":"seeds-ask-native-port/v1","enabled":false,"admitted":null}
```

An admitted envelope additionally pins endpoint, project graph, store identity,
machine principal, custody directory/UID, dedicated writer credential file,
code root, Node SHA-256, all seven asset SHA-256 values, reader identity/query
hashes/three registrations, and optional public CA artifact/hash. The loader
requires a root-owned private profile and root-owned non-writable code and
parent directories. The writer runs as the non-root custody UID. There is no
default endpoint, ambient bearer fallback or development override. Machine
configuration requiring a different session-signature policy refuses rather
than bypassing that policy.

Before the one Seeds CAS, custody reserves a stable filename derived from the
store identity and full intent. The directory must belong to the current
non-root UID with mode 0700. `openat(O_EXCL|O_NOFOLLOW)` creates mode 0600;
writing, file fsync and directory fsync precede sending. Code updates do not
create a new slot. Any failure retains the reservation. Never delete/reset an
uncertain intent to retry. `--reconcile` reads no writer credential, reserves
nothing and sends nothing; it re-verifies native evidence and reads the existing
settlement.

This port keeps the **full Seeds write record inside the project graph in the
same atomic CAS**, because its intended scoped writer has only that graph's
update authority. Ordinary CLI writes retain the existing sibling
`#provenance` layout. A bounded audit reader joins the exact item and produced
revision to the settlement source and requires one complete write record. This
storage placement is staged source behavior: production use requires a separate
operator storage-authority decision. It grants no server permission and creates
no owner lane. The exact scoped writer credential/grant, production profile,
immutable bundle custody and real acceptance remain separate admission gates.

The native fixture can join an actual separately spawned Rikuy verifier and
native receipt to a real disposable in-memory Seeds atomic transaction. Its
explicitly ignored test is invoked by Rikuy's native qualification using
`SEEDS_NATIVE_PORT_TEST`, the built Seeds library-test executable. Only that
test constructor bypasses production root deployment selection; the release
command exposes no override. That fixture does not qualify production root
installation or the remote scoped writer grant. The separate real-server audit
control exercises project-local audit read-back without changing ordinary CLI
placement.
