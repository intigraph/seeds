# Revision conflict: before and after

The recording runs the same two-client scenario against exact base `fc3c497dc5359b35ecee7d6a29c27e8bf1c02e60` and the feature source in this change. Both builds already refuse the stale guarded update and preserve the winner. The change exposes the expected/current revisions and bounded current-versus-requested field previews, so the caller can reread and explicitly rebase. `--force` does not defeat the revision fence.

[Recorded output](output.txt) · [real timing](output.timing) · [driver](run-demo.py)

```sh
scriptreplay --log-out docs/probes/pr-demos/revision-conflicts/output.txt \
  --log-timing docs/probes/pr-demos/revision-conflicts/output.timing
```

To reproduce, build separate base and candidate executables, set `SEEDS_DEMO_BEFORE` and `SEEDS_DEMO_AFTER` to their absolute paths, then run `python3 docs/probes/pr-demos/revision-conflicts/run-demo.py`. The driver uses independent temporary local fixtures, two actual CLI reads, an actual winning edit, a stale edit, and an explicit reread/rebase. It asserts both the conflict status and the unchanged winner. No production store is used.

The candidate binary used for this recording has SHA-256 `bdcabd9f84db2e71862d9d39b42c7d3b244eee15db7521857d194ed0bce6361c`; the base binary has SHA-256 `d86298dc30475c772170aca3ab3aa977a3771baadc0a919e13ef0c5a9b0a03b4`. These identify the copied executables exercised by the recording; shared Cargo target directories were not treated as immutable artifacts.

The recording covers local conflict diagnostics. Separate real loopback integration controls exercise uncertain remote replies: a committed operation is reconciled once, and an identical post-state written by another operation cannot establish success without this operation's provenance marker. Installed release acceptance remains separate.
