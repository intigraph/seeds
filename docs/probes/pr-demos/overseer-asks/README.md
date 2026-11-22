# Overseer Ask records

On two disposable local stores, the released CLI rejects the new Ask fields
(exit 2). The candidate creates an Ask linked to ordinary work. Default list,
count and ready each still contain only that work; `list --type ask` returns
the Ask, and its exact record retains the linked item, fallback and expiry.
Closing the Ask leaves the ordinary work ready. This is source behavior, not
installed or browser acceptance.

The actual terminal recording is [output.typescript](output.typescript), with
[timing.txt](timing.txt) and a [readable transcript](output.txt). Replay with:

```sh
scriptreplay --log-out output.typescript --log-timing timing.txt
```

Reproduce with independently copied binaries:

```sh
DEMO_BEFORE=/path/to/released-sd DEMO_AFTER=/path/to/candidate-sd python3 demo.py
```

Before: released `sd 0.25.0`; candidate: this PR's source over base
`e47bf578c9efa4adde5d226c81d68cead2702f1d`. The copied candidate's required
behavior is checked by the reproducer. No network or real ledger is used.
These small recording artifacts are retained with the source for review.

The local recipient shape profile pin changes with the new optional metadata
constraints. An incoming bundle cannot select or install that policy; recipients
still need an explicitly installed matching local profile.
