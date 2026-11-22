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
