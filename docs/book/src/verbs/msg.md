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

With `--key`, the id is derived from the key alone, so a resend after a lost
response names the same message and writes nothing new (`already sent`).
Without a key, the id comes from recipient, send time and body.

When the server's response is lost, `sd msg send` reads the window back and
reports the outcome; if it still cannot tell, it says UNKNOWN and that a resend
with the same key is safe.

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
