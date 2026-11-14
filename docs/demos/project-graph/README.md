# Project-graph migration demonstration

The same two-item legacy ledger is read on base and candidate. Base refuses the
new migration command; candidate preserves every exported fact and item view,
then plain Quipu loads it after explicit recipient profile setup. The demo also
checks key/database staging exclusions and retains the profile-less refusal.

```bash
python3 docs/demos/project-graph/demo.py \
  --base /path/to/base/sd --candidate /path/to/candidate/sd \
  --quipu /path/to/quipu --profile shapes/seeds.shapes.ttl
```

Use the reviewed profile from Seeds `v0.22.1`, with the digest in the storage
modes documentation. Every write targets a disposable local fixture with an
empty credential environment. This is source qualification, not an installed
fleet migration. Plain Quipu's missing-profile refusal currently reports
quarantine; naming the setup step before staging is a separate producer gap.

The retained transcript is actual command output. A terminal recorder was not
available on the authoring host; it is not a video or a simulated result.

The [atomic publication regression](atomic-publication.txt) shows the reviewed
failure and corrected behavior. Migration prepares the identity, graph and
Seeds config together, then publishes them with one directory rename. A
post-scaffold filesystem failure leaves legacy routing active.
