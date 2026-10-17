# Rank ordering demonstration

Trigger: insert `moving` before `right` within priority 1. The base CLI refuses
`--before`; the candidate assigns `1.5` and `ready` returns left, moving, right.
Neighbor records and all due dates stay unchanged.

This is an actual output-only terminal recording against disposable local
fixtures, not a deployment. The base is the installed sd 0.4.2 artifact (its
source commit is unknown); the recording identifies its SHA256. Candidate code
is commit `c37d6ff`, built locally from the source based on `62942a56` (0.5.0),
with its actual binary SHA256 recorded. This is author-build evidence, not an
independent release build. Initial creation-time ties may order IDs differently;
the explicit ranks determine the demonstrated final order.

Download [output.log](output.log) and [timing.log](timing.log), then replay:

```bash
scriptreplay --log-out output.log --log-timing timing.log
```

The output is also a readable transcript; no inline player is assumed.
To reproduce, set `RANK_BASE_SD` and `RANK_HEAD_SD` to the two binaries and run
`python3 demo.py`. Both stores and HOMEs are temporary, with remote Seeds
configuration removed. The small recording is retained beside the reproducer.
