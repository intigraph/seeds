# CLI command-source demo

The same native read-header builder previously used the component source
`seeds`. Inside the CLI's parsed-command scope it now emits `sd-show`, without
including the item argument. The underlying operation remains `query`.

- Base: `2635e72b6b69a6684a673a0beee5d11d9fa8c294`.
- Demonstrated source: `18ef5ba2e5336a6c90b24d716a4d9a910eff0be7`.
- Target: private native library fixture, not an installed SDK or live store.
- Reproduce: `cargo test --lib demo_command_attribution_before_and_after -- --nocapture`.

The [terminal recording](output.txt) and [timing](timing.txt) can be replayed
with `scriptreplay --log-out output.txt --log-timing timing.txt`. The output is
also a readable transcript. Build diagnostics are separate from the recording;
the recorded command exited zero.

Separate built-CLI loopback tests cover all eight existing requests for `show`,
including its connection and preflight. The request count is unchanged. They
also prove an explicit outer `st-anchor` declaration survives every request,
unknown actors remain unknown, and task/session declarations are preserved.
This demonstrates metadata propagation, not a query reduction, release,
installation, server-log retention, or complete census coverage.
