# Missing input, explicit outcome

A missing comment file keeps human output unchanged: exit 2 and no stdout.
`--result-json` keeps exit 2 and adds MISSING_INPUT, INPUT, NOT_WRITTEN, exact
task/graph identity and an invocation ID. The final exact read finds zero
comments, a positive control that this attempt submitted nothing.

Recorded implementation: `26911896a29c3fedc8bcadeb08d2c3c4bcf9d046`
(base `4cf5b2f5ddb6d8e8473dd9f19c5ad2ec48a11764`). This compares preserved
human mode with the new mode on the recorded implementation. No independent
base executable run is claimed; the new option has no base equivalent. The
source command and schema controls separately cover malformed replies, HTTP
failures and a real committed comment with a lost reply and failed readback.

Reproduce with a binary built from the reviewed source:

```sh
DEMO_SD=/path/to/reviewed/sd python3 docs/demos/tracker-result-demo.py
```

The script creates an embedded disposable store with an isolated HOME. It
performs no network, deployment or credential action and never opens a real
board. It demonstrates source behavior, not installed acceptance.

[Output recording and readable transcript](tracker-result.output) and
[timing](tracker-result.timing) are retained in this demos directory. Both were
recorded by util-linux `script`; playback was verified. Download both and use:

```sh
scriptreplay --log-out tracker-result.output --log-timing tracker-result.timing
```

This is a replayable terminal recording, without an inline video embed.
