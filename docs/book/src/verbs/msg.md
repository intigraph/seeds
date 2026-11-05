# msg

Crew mail, kept out of the board. A message is not a seed: it has no status,
priority or assignee, and no `list`, `ready`, `count`, `show`, pendant, export
or sync ever sees it.

```text
sd msg send --to <name> [--from <name>] [--kind note|alert] [--key <key>] [--about <seed>]... <text>|-m <text>|-f <file>
sd msg list  [--to <name>] [--unread] [--weeks <n>]
sd msg count [--to <name>] [--unread] [--weeks <n>]
sd msg show <msg-id> [--weeks <n>]
sd msg read <msg-id>... [--weeks <n>]
```

## Where messages live

Each message is a `schema:Message` in the window graph of its send week,
`<project graph>/messages/<YYYY>-W<ww>` (ISO week), and it never moves. The
window is not under `#seeds-…`, which is why board reads cannot see it. Each
send also writes the usual `seeds:Write` provenance record, in the window's
own `#seeds-provenance` graph.

| field | predicate |
|---|---|
| id | `schema:identifier` (`msg-<hex>`) |
| preview | `rdfs:label` (first line, at most 120 characters) |
| recipient, sender | `schema:recipient`, `schema:sender` (principal IRIs) |
| sent | `schema:dateSent` (`xsd:dateTime`) |
| body | `schema:text` |
| kind | `seeds:messageKind`: `note` or `alert` |
| idempotence key | `seeds:deliveryKey` |
| refers to | `schema:about` (seed IRIs; a message never carries work) |
| read | `schema:dateRead` (`xsd:dateTime`), written by `sd msg read` |

## Idempotence

With `--key`, the id is derived from the key alone. A per-project binding in
`<project graph>/message-bindings` links that id to its original week with
`schema:isPartOf`. The binding, message and provenance commit together. Remote
writes guard the binding under the server's writer lock; local CLI writes hold
the store's exclusive lock for the whole verb. Two sends in different weeks
therefore compete for the same slot, rather than each claiming its own week.

A retry returns the original recipient, send time and window, even outside the
listing's recent-week horizon. Reusing a key with a different sender, recipient,
body, kind or seed references refuses; it never replaces the original message.
Without a key, the id comes from recipient, send time and body.

Bindings are retained independently of read marks and window retention. Do not
expire or archive the binding graph with a week window: forgetting the binding
would allow an old delivery to be written again. If the original window becomes
unavailable, resolving its binding fails; it is not permission to resend into a
new window. No existing production messages are migrated by this command.

When a response is lost, `sd msg send` resolves the binding and reads the original
message rather than searching only the retry's window. An absent, malformed,
ambiguous, truncated or unreadable receipt stays UNKNOWN; the write is not
replayed automatically.

## Reading

`list`, `count` and `show` read the windows of the last `--weeks` ISO weeks
(default 2, this week included), oldest message first. A window that does not
exist holds no messages; a malformed message or a truncated answer refuses the
listing instead of returning part of it.

`sd msg read` appends `schema:dateRead` once; a message already read keeps its
first mark, so repeating it is safe, and nothing is deleted or closed. Reading
is not acknowledgement: a separate ack, expiry and escalation come with the
next phase of the design (shantytown `docs/message-graph.md`). `--unread`
lists or counts only messages without a read mark.

The remote gate admits the `schema:Message` class through
`seeds:RemoteMessageShape` in `shapes/quipu-gate.shapes.ttl`; a server that has
not loaded that shape refuses the write.
