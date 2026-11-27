# Duplicate intake

`sd create` compares title and description with active seeds before writing.
Canonical-name matches at **0.99**, or configured embedding matches at **0.92**,
refuse with exit 5 and existing IDs:
comment on those seeds instead. Active means open, hooked, in progress, blocked,
or deferred in this project, including its ephemeral graph. Closed and archived
work does not block a new issue. This is a similarity gate, not automatic merging.

Local stores reuse Quipu's canonical-name resolver (including Jaro-Winkler)
over the active corpus. They have **no embedding provider**. Remote stores also
call the configured Quipu server's read-only `/resolve`, which combines name
matching and its configured embedding search over title plus description.
The client filters those candidates to this project's active seeds. Server
errors, invalid responses, partial vector scope, and results reaching the 1,000
candidate cap refuse quick and bulk creates: they are incomplete coverage, not an empty match.
`sd create` records them as UNKNOWN instead (see [Hint mode](#hint-mode)).
The thresholds are similarities, not calibrated probabilities. A retained
seven-day canonical replay flagged 119/533 proposals at 0.92; inspecting those
found different identifiers in shared templates. The 0.99 name floor retained
88/533 and removed those inspected template-only matches. Embedding coverage
and original historical text are separate from that estimate.

If the work is distinct, explicitly reject each candidate and say why:

```sh
sd create "Repair parser authentication" \
  --new-despite sd-existing --because "Separate deployment and failure cause"
```

`--new-despite` is repeatable. It must name current candidates; `--because` must
be nonempty. Every unmatched candidate still refuses the create. One transaction
creates the seed, related edges and an audit comment containing the candidate
IDs, scores, match methods, threshold, reason, actor and time. Normal writer
attribution records the source automatically. There is no generic skip flag.

Quick creates (`sd q`) and bulk markdown creates also check for duplicates;
intentional duplicates use `sd create` with the explicit override. Bulk name
matches within the file refuse the entire batch. Dry runs do not write.
Existing workflow run/step retries still return their original seed unchanged.
The gate is client admission, not a server uniqueness constraint: simultaneous
remote creators can both pass before either commits. It does not promise serial
creation across clients or that an embedding index has indexed every source fact.

`examples/intake_replay.rs` replays a JSON corpus offline through the same
canonical-name resolver. Its output estimates canonical-name refusals on the retained inputs, without
embeddings. Current titles/descriptions and recorded close times cannot reconstruct
unrecorded past edits, reopenings, or deletions. Live creates/closes rates are
installed acceptance, measured after normal deployment, not source-test results.

## Hint mode

Below the refusal gate, `sd create` also **hints** (aegis-8dda.12). It runs the
same Quipu resolver over a wider corpus: WorkItems that are active or were
closed in the last **30 days**, excluding inbox pointers (`inbox:`/`mail:`
titles, the `inbox` label), ephemeral seeds and machine records (`ops`, `ask`).
Canonical-name matches at **0.90** (Jaro-Winkler included) and the remote
embedding candidates become hints. Nothing is merged.

The hinted side is filtered the same way. A new inbox pointer, ephemeral
seed or machine record gets no hints and records no hint facts, so it never
appears in `sd dupes` and `--strict` never refuses it for a hint. `sd dupes`
also skips any such seed that already carries a hint.

The create succeeds (exit 0). The top three candidates are printed on stderr
with id, status, score and title, and recorded on the new seed:

| Predicate | Value |
|---|---|
| `seeds:possibleDuplicateOf` | one IRI per candidate |
| `seeds:duplicateHintState` | `hinted`, `none` or `unknown` |
| `seeds:duplicateHint` | JSON: threshold and Quipu `resolution_hints`-shaped candidates (`iri`, `score`, `matched_on`) |

If the resolver fails (a timeout, for example), the state is `unknown` and the
warning says UNKNOWN. That is never reported as "no duplicates". In hint mode
the failure no longer refuses `sd create`; this is the item's acceptance test
("a Quipu timeout leaves the hint UNKNOWN and the create proceeds with a
warning"). The resolver call is shared, so the failure also skips the remote
(embedding) half of the refusal gate, and the warning says so in those words.
The local canonical-name gate (0.99) is unaffected. Use `--strict` to keep
the create fail-closed. Quick and bulk creates, which have no hint mode,
still refuse.

The closed half of the corpus is read with a bounded query (seeds closed at
or after the window start), never a snapshot of every closed seed. Remote
canonical-name candidates are floored at 0.90, the same as local ones.

- `--strict` refuses (exit 5) when there is any hint or the state is UNKNOWN.
- `--silent` still prints the duplicate warnings on stderr; it silences
  only stdout and the directive hints. A duplicate warning, and above all an
  UNKNOWN one that says the refusal gate was degraded, is not advisory noise.
- `--distinct-from <id>` (repeatable; each id must exist) records `quipu:distinctFrom` on the new
  seed. That pair is never hinted again, whichever side carries it.
- A title or description edit (`sd update`) re-runs the hints and replaces
  the recorded hint facts. The edit is already written and there is no gate,
  so a failure is only a warning that the hints are UNKNOWN. Closed seeds, inbox pointers, ephemeral seeds and machine
  records are not re-hinted.

`seeds:possibleDuplicateOf` and `seeds:duplicateHintState` are seeds' stopgap
terms, like `seeds:workflowRun`. They are to be proposed to camayoc.

### Grooming: `sd dupes`

`sd dupes` lists the open hint pairs: the hinted seed is not closed, and
neither side records `distinctFrom`. `--json` returns an array of
`{id, title, status, duplicate_of, duplicate_of_title, duplicate_of_status}`
(pinned in `tests/json_schema.rs`; the last two are `null`, never absent, when
the target is unreadable).

```sh
sd dupes resolve <id> <of> --duplicate  # close <id>, outcome superseded, reason "duplicate of <of>"
sd dupes resolve <id> <of> --distinct   # record quipu:distinctFrom, drop the hint
```
