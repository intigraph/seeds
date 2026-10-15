# External blocker notification contract

Seeds evaluates prerequisites and stores unlock intent. The shared notifier
owns delivery, its retained receipt ledger, and scheduled invocation.

Run `sd blockers reconcile --json` once before reading events. A failed or
unreadable prerequisite remains unresolved. Unchanged observations make no
write. An actual feature-owned blocked-to-open transition increments the
persisted readiness generation and creates an intent in the same seed write.
Manual blocked, deferred, in-progress and closed states are preserved.

`sd blockers events --json` returns persistent, currently eligible pending
intents. Unknown delivery must leave them pending. The event ID survives
ordinary item and assignee edits, consumer restart, and repeated reads.
Temporary absence does not authorize the consumer to forget an event ID.

Example response:

```json
{
  "events": [
    {
      "event_id": "seeds:unlock:example-1:1",
      "issue_id": "example-1",
      "assignee": null,
      "ready_generation": 1,
      "observed_at": "2026-10-14T12:00:00Z",
      "receipt": null
    }
  ]
}
```

Map each row to the shared consumer's event shape:

```json
{"kind":"unlock","id":"example-1","episode":"seeds:unlock:example-1:1","recipient":null}
```

A null recipient selects the notifier's configured administrator. After its
retained delivery receipt is verified, acknowledge using:

```sh
sd blockers ack example-1 seeds:unlock:example-1:1 --receipt retained-receipt-1 --json
```

The ACK records the receipt with a revision check. Repeating the same ACK is a
no-op; a conflicting receipt refuses. If ACK delivery is uncertain, reread the
pending events and retain the consumer receipt before any retry. A receipt is
not permission for a second notification. Once acknowledged, the intent remains
in the seed's history and is absent from pending events.

Successful reconcile response (transaction IDs are examples):

```json
{"changed":[{"issue_id":"example-1","tx":123}]}
```

An unchanged poll returns `{"changed":[]}`. A successful new ACK returns
`{"tx":124}`; a repeated identical ACK returns `{"tx":null}`.
For an uncertain ACK, read the exact retained event independently of readiness:

```sh
sd blockers receipt example-1 seeds:unlock:example-1:1 --json
```

```json
{"event":{"event_id":"seeds:unlock:example-1:1","issue_id":"example-1","assignee":null,"ready_generation":1,"observed_at":"2026-10-14T12:00:00Z","receipt":"retained-receipt-1"}}
```

An error is UNKNOWN, not an absent receipt. A null receipt means the exact
event is readable but not acknowledged. Retry the byte-identical ACK only
after that readback; retain the notifier's receipt and never resend delivery.

The in-memory core test `external_unlock_outbox_is_persistent_receipted_and_rearms`
covers a prerequisite in another registered graph, pending and complete states,
repeat polls, nullable routing, retained ACK, conflicting ACK, stale completion
reads and a second readiness generation. Installed scheduled delivery acceptance
is a separate remaining gate; this document does not claim deployment.
