# Command-level request attribution

The recorded real CLI requests change from `X-Quipu-Client: seeds` to
`seeds/show`. A different subject argument keeps the same bounded label.
All three invocations intentionally return exit 3 for a missing synthetic subject.

The before binary is the retained actor-compatibility proposal at `6f9300ad`;
its actor changes are not exercised by this read-only fixture. It is not a
build of the current base. The after binary is built from `ac0e758a` after the
command-label change and current-main integration. Exact binary SHA256 values
appear in the transcript. This fixture proves actual HTTP headers, not real
server authentication, deployment, release, or canary acceptance.

The output and timing were captured together with util-linux `script`.
`transcript.txt` only normalizes terminal CRLF to LF for reading.

Replay the original bytes:

```sh
gzip -dc output.txt.gz > /tmp/command-client-output.txt
scriptreplay --log-timing timing.txt --log-out /tmp/command-client-output.txt
```

Reproduce with two existing CLI binaries (Python standard library only):

```sh
python3 reproduce.py /path/to/before/sd /path/to/after/sd
```
