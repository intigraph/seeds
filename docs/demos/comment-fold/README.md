# Bounded comment display

A disposable local task gets seven numbered comments. The before binary shows
all seven by default. The candidate shows positions 3–7, with total 7 and folded
2. Its explicit full-history view and `comments list` both still return all
seven. Nothing is deleted or rewritten.

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
installation or production credentials. The recording pins both executable
SHA256 values and versions. Before is the installed 0.11.1 artifact; its exact
source revision was not exposed by `--version`, so no source-SHA claim is made.
After is the 0.12.0 candidate built from the source retained beside this demo;
[source digests](source-sha256.json) bind those files. The PR identifies its
exact head. This proves CLI behavior, not installed fleet acceptance.
