# Operations log

Machine records can leave the work board without losing IDs or comments.
Close an `ops` record with an explicit disposition first, then plan a move:

```sh
sd archive --operations-log --id project-example --closed-before 2030-01-01T00:00:00Z --protect protect.json --batch 1
```

The destination is the project graph with `#operations-log` appended. Only the
explicit IDs are considered; ordinary work, open records, protected dependency
targets and existing destination records are refused or retained. Add `--execute
--log move.jsonl` to move the selected records. The SDK logs each intent before
sending one signed update and verifies exact source/destination facts afterward.
An uncertain result stops the run; inspect both graphs before continuing.

Restore with `sd unarchive --operations-log --from-log move.jsonl --execute`.
The inverse retains the recorded closed disposition. Reopening work is a separate
operator action. Do not mix archive and operations-log intent files.
