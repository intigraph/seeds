# External blocker before and after

[Replayable terminal recording](assets/external-blockers.cast) · [Actual transcript](assets/external-blockers-transcript.txt)

The same `dep add --blocker` invocation is rejected by the installed sd 0.3.0 baseline (exit 2), then accepted by the candidate. Each run uses an isolated local store with two named graphs. A prerequisite initially open in the foreign graph keeps the local item out of ready. Closing that prerequisite and reconciling once makes the item ready and produces one retained unlock intent; a second poll makes no write and returns the same event ID.

Source proposal base: `3003e779bf9c86beaf561868551dc620d75ac551`. Candidate product source: `c2ceef2577a6c33281ad6605e9c4cc12cd530824`. The baseline version probe reports sd 0.3.0 and does not expose a commit SHA. The recording is a candidate fixture demonstration, not installed acceptance or notification delivery. The shared consumer owns the sender and receipt verification.

Repeat with a local store and two graph IRIs, create one seed in each, then run:

```sh
sd --store demo.db --graph urn:demo:work dep add ITEM --blocker FOREIGN_IRI --blocker-graph urn:demo:foreign --done-when closed
sd --store demo.db --graph urn:demo:work ready --json
sd --store demo.db --graph urn:demo:foreign close FOREIGN_ID --reason fixture-complete
sd --store demo.db --graph urn:demo:work blockers reconcile --json
sd --store demo.db --graph urn:demo:work ready --json
sd --store demo.db --graph urn:demo:work blockers events --json
sd --store demo.db --graph urn:demo:work blockers reconcile --json
sd --store demo.db --graph urn:demo:work blockers events --json
```

Run from a disposable working directory as well as an isolated HOME: the CLI can synchronize a pendant in its current working directory. The corrected recording leaves the source checkout clean. Its actual output and timing were retained and replayed; the cast contains those recorded chunks and timings.
