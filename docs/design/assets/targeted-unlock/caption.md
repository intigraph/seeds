# One-target reconciliation

Replay with `scriptreplay --log-out output.log --log-timing timing.log`.
The recording shows actual CLI calls against isolated disposable stores.
Baseline: installed sd 0.11.1 (SHA256
`0126a41576bc880f6cf3fb34e85396821648ba3da9ae9ce3fba8364dcfe075c2`),
which rejects the target argument. Candidate source: `4bbfc9e`, based on
`fc3c497d` (0.12.0). This is a CLI comparison, not a deployment claim.

Two feature-managed seeds become eligible after their external blockers are
removed. Targeting the first opens it while its sibling stays blocked. Replay
writes nothing and returns the same single pending event. This demonstration
uses blocker removal as its transition; foreign-node completion and real
notification delivery remain separate installed acceptance checks.

Reproduce with `python3 demo.py /path/to/baseline-sd /path/to/candidate-sd`.
The baseline must reject targeted syntax; the candidate must support it.
Readable fallback: [output.log](output.log). Output and timing are small,
sanitized repository artifacts retained with this change; no inline player is
promised.
