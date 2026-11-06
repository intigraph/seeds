# Every live message window is visible

This demonstration uses the actual native candidate CLI at code revision
`7476dc88a19a4277fdeea6fae8a35a60de4af54a` and a disposable local store. Quipu
seeds one message dated January 2000 into its registered week window. The
existing recent-week scope returns zero; explicit `--all-windows` returns the
message, marks it read and then counts zero unread. Both scope controls use the
same candidate binary; this is not a separately built baseline comparison.

Run with already-built executables:

```sh
python3 docs/demos/all-live-windows/demo.py --sd /path/to/sd --quipu /path/to/quipu
```

The [transcript](output.txt) contains actual calls, exit codes, responses and
executable hashes. It is readable output, not a timing recording: no recorder
was used and replay timing is unavailable. The small artifact and caption are
retained in this repository for review. This performs no network calls or
production writes. No message-graph config flip, expiry or deployed-service
acceptance is implied. The separate message and registry controls cover other
projects, frozen/attached windows and malformed/incomplete registry refusal.
