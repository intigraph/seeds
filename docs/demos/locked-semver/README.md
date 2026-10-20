# Locked API comparison recording

Source-fixture replay of real rustdoc JSON generated with Rust1.98.1 and the exact original release lock. Original source: `05512a7`; actual release: `8d5d10b`, tag `seeds-ai-v0.6.1`; cargo-semver-checks0.50.0. Actual compare calls show release-self exit0, original167 API changes exit100, and a private-rank-module mutant exit100. The original unlocked compile separately reproduced E0063; this recording replays JSON comparisons, not the long compile. Recorder command metadata is redacted; payload/timing are real.

Download [output.txt](output.txt) and [timing.txt](timing.txt), then run `scriptreplay --log-out output.txt --log-timing timing.txt`. No inline video player is promised. The source/lock reproduction and live proof command are in the [book](../../book/src/semver-baseline.md). These small source-only artifacts are retained in Git with this repair.
