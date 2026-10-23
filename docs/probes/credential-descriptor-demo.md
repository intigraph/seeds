# Selected credential descriptor demo

An inherited inline token must not silently override an explicitly selected
command credential. This recording uses inert strings and a loopback fake server
that refuses its first authenticated write. No real ledger or provider is used.

Before: the installed sd 0.4.2 ignores the new descriptor metadata and sends the
inherited fixture token. Its source revision is unknown; this is deliberately
**not** represented as a build of the PR base. After: the proposal based on
`fcc06c4d1c59b59e3d8254185f20753114c5de25` sends the selected fixture token,
refuses malformed selection without making any request, and uses ordinary
inputs only on a later deselected command. The commit containing this caption
binds the proposal source; the recording proves a private candidate, not delivery.

[Readable transcript](credential-descriptor-demo.txt) and
[timing file](credential-descriptor-demo.timing) form a terminal recording.
Playback was checked with util-linux `scriptreplay`:

```sh
scriptreplay --log-out docs/probes/credential-descriptor-demo.txt \
  --log-timing docs/probes/credential-descriptor-demo.timing
```

To reproduce, set `CREDENTIAL_BEFORE_SD` to an installed 0.4.2 binary and
`CREDENTIAL_AFTER_SD` to a freshly built proposal binary, then run
`python3 docs/probes/credential-descriptor-demo.py`. The normal test command is
`CREDENTIAL_SD=/path/to/candidate/sd python3 tests/credential_descriptor.py`.
The small text recording lives with this source so it survives branch deletion;
no external media host or inline playback is assumed.

This native descriptor primitive does not implement a provider or a session
launcher. A launcher must deliberately select/deselect each child, sanitize its
inherited credential inputs, preserve private descriptor lifetime and enforce
provider policy. Installed CLI, actual authenticated-principal and SDK consumer
acceptance remain separate.
