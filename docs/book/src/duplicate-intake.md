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
candidate cap refuse creation: they are incomplete coverage, not an empty match.
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
