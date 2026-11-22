# Operations records

The released CLI refuses `--type ops`. The candidate accepts it, keeps the
record in an explicit Operations log, and excludes it from backlog, ready
work and work statistics. Direct reads and normal closure remain available.
This does not migrate legacy records or change producer configuration.

Run with two independently built binaries:

```sh
python3 docs/probes/pr-demos/ops-records/demo.py --before /path/to/released-sd --after /path/to/candidate-sd
```

The demonstration uses disposable local stores and synthetic records; it makes
no network requests. `output.txt` records the actual replay.

Base: released v0.24.0, commit `e304439af2fcc41031338c87fd7c165bca24d2c3`.
Candidate: this PR's source head (including the owner-readiness exclusion).
The binaries are copied into a private verification directory before recording;
the candidate is a source build, not an installed release.

Download `output.log` and `timing.log` together and replay with:

```sh
scriptreplay --log-out output.log --log-timing timing.log
```

These small terminal artifacts are retained alongside the reproducer in Git.
`output.txt` is the readable transcript; no inline video playback is claimed.
