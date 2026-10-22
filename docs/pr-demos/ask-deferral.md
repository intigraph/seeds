# ASK deferral refusal

The [recording](ask-deferral.typescript) compares sd0.6.6 with this candidate
against separate, private local stores. All four governed labels and the title
prefix previously deferred successfully (exit0); each now refuses (exit5).
The ordinary technical item still defers successfully in both versions.
There are no production calls, credentials or changes to a shared ledger.

Reproduce with `BEFORE_SD` and `AFTER_SD` pointing at the two executables, then
run `python3 docs/pr-demos/ask-deferral.py`. The script sets a private HOME and
working directory for every SDK process, so repository pendant configuration
cannot participate. Replay with
`scriptreplay --log-timing docs/pr-demos/ask-deferral.timing --log-out docs/pr-demos/ask-deferral.typescript`.

The paired tests also exercise both `update --status deferred` and `update
--defer`, mixed-batch atomic refusal, same-request classification changes and
unchanged revisions after refusal. This is CLI/engine proof; it does not assert
server-side enforcement or installed fleet acceptance.
