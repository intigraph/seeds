# Pinned HTTPS CA demonstration

The same isolated CLI input changes from an unknown `ca_file` config error to a
successful TLS `/health` request using the configured CA. The fixture has no
signing key: identity then correctly refuses with `identity requires an enrolled
signing key`. Foreign CA, wrong hostname, missing CA and changed PEM bytes all
refuse before an HTTP request. No server enrollment or native account is involved.

[Output](output.txt) and [timing](timing.txt) are a replayable terminal recording:

```sh
scriptreplay --log-out docs/demos/pinned-ca/output.txt --log-timing docs/demos/pinned-ca/timing.txt
python3 docs/demos/pinned-ca/demo.py --before /path/to/base/sd --after target/debug/sd
```

The retained before binary reports `sd 0.11.1`; the candidate reports `0.13.0`.
This is a binary behavior comparison, not a claim that the before binary is the
exact parent commit. Parent source `b50b9c5` also lacks both CA settings and the
custom TLS client. The PR body identifies the exact candidate commit. All server,
CA and key material is synthetic localhost test data under `tests/fixtures/tls`.
Temporary fixture paths in output are replaced with `$FIXTURE`; actual exit codes,
responses and timing are retained. Playback requires downloading both files and
`scriptreplay`; no inline player is promised. These small files remain in Git
with this demo. This proves candidate TLS transport, not deployed identity.
