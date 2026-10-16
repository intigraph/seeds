# Bulk creation through a transient gateway outage

This is a replayable terminal recording of actual `sd` processes against a
disposable local Quipu server. A local HTTP proxy injects one health 503 and
one query 502. It never injects a write failure or touches a production endpoint.

The same ten-item Markdown file describes an epic and nine children, with
Parent `@1`, assignees and labels. The installed 0.4.1 comparison exits after
the health failure and sends zero batch updates. The 0.4.2 candidate recovers
from both transient responses, creates all ten items with nine correct parent
links, and sends exactly one batch `/update` in 2.387 seconds.

- [Recording output](output.txt) and [timing](timing.txt).
- [Reproducer](reproduce.py): set `SEEDS_DEMO_BEFORE_SD`,
  `SEEDS_DEMO_AFTER_SD` and `SEEDS_DEMO_QUIPU_SERVER` to the three executables,
  then run `python3 reproduce.py`.
- Download the output and timing files together and replay with
  `scriptreplay --log-out output.txt --log-timing timing.txt`.

The recording's source base is `525ab6ad6edffc9eba8af1d12539d2179f728a33`.
The comparison is the installed 0.4.1 release binary; its exact build commit
is not claimed. This demonstrates candidate behavior and atomic batch traffic,
not a published release, a fleet installation, or live-board performance.
The repository retains this small recording with its reproducible fixture.
