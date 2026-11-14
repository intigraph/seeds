# Exact message lookup demo

A private loopback mailbox has a valid legacy target and a malformed neighbour.
The fixture rejects broad partition reads to model the old capped-read failure.
The released sd 0.20.1 exits 1; the candidate returns only the target, exit 0.
A malformed target still exits 1 and names itself. This fixture demonstrates
lookup isolation; it does not reproduce a live server's row cap or prove deployment.

Download [output](output.log) and [timing](timing.log), then replay:

```sh
scriptreplay --log-out output.log --log-timing timing.log
```

The recording uses source based on `4aeb38a`, before the final defensive
validation of direct backend IDs (the recorded valid ID is unaffected).
The candidate is a local source build, not an installed release. The retained
released binary is not a rebuild of the exact PR parent. These small terminal
artifacts live with this probe; there is no external player or inline video.

Repeat with two explicitly selected binaries from the repository root:

```sh
BASELINE_SD=/path/to/released/sd CANDIDATE_SD=/path/to/candidate/sd \
  python3 docs/probes/pr-demos/exact-message-show/run.py
```

Both children use temporary HOME/config directories and a small environment
allowlist. No production server, credentials or mailbox writes are used.
The full wire controls run as part of `just check`.
