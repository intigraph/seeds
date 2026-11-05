# Bounded comment display

A disposable local task gets seven numbered comments. The pre-review candidate
f6d8b76f incorrectly exposes only five in default JSON, hiding the oldest evidence.
The repair retains all seven in default JSON, including the oldest evidence used
by receipt-dedup readers. Text remains bounded to positions 3–7 in both versions.
Explicit JSON `--comments-limit 2` still reports total 7 and folded 5. Full-history
controls return all seven. Nothing is deleted or rewritten.

[Terminal output / readable transcript](output.log) and [timing](timing.log)
are a util-linux recording, retained here as small sanitized Git artifacts.
Download both, then replay with:

```sh
scriptreplay --log-out output.log --log-timing timing.log
```

Playback was checked; there is no inline player. Repeat from the repository
root with two supplied binaries:

```sh
COMMENT_FOLD_BEFORE=/path/to/base/sd COMMENT_FOLD_AFTER=/path/to/candidate/sd \
  python3 docs/demos/comment-fold/run.py
```

Target: synthetic local stores and private fixture homes; no service, network,
installation or production credentials. The recording pins executable SHA256
values and versions. Before is the f6d8b76f pre-review candidate; after is built
from the repair files bound by [source digests](source-sha256.json). The PR names
the exact repaired head. This proves candidate CLI behavior, not fleet acceptance.
