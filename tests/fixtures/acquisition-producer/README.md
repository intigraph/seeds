# Bounded acquisition fixture

This is an independent, unpublished test workspace. It builds the exact reference
producer at commit `0f23ef3be3cc6ed32d62dacd4cf7c0986fc803bb` without changing the
SDK's runtime dependencies. It is a synthetic loopback server, never a production
server or an installed SDK adapter. Only one test driver/server runs at a time.

Linux is required for the retained-provider fencing control.

The SDK test uses its existing HTTP transport and Seeds parser/model. Verification
is explicitly delegated to the pinned producer's actual `Payload::verify` method
in this helper process; the test DTOs are not a replacement verifier. The normal
remote historical-export refusal remains enabled.

Build both workspaces, pin their source and executable SHA-256 identities before
execution, then run the opt-in test with an absolute helper executable path:

```sh
cargo build --locked --manifest-path tests/fixtures/acquisition-producer/Cargo.toml
SEEDS_ACQUISITION_FIXTURE_DRIVER=/absolute/path/to/seeds-acquisition-fixture \
  cargo test --lib bounded_acquisition_g2_g3_then_g4_g6 -- --ignored --exact \
  native::remote::acquisition_fixture::bounded_acquisition_g2_g3_then_g4_g6 --nocapture
```

Run with a temporary HOME and inspect the normal watched user resources. The
fixture creates a new owned temporary directory; it refuses an existing one,
registers only synthetic graphs, and removes only that directory after the child
exits. It binds loopback on an ephemeral port. It does not read service credentials
or select a configured server. Source receipts are verified before artifact writes;
raw receipt bytes and their original hash stay separate from fixture artifact
metadata. The helper's mutation routes are private test controls, not API proposals.

This proves only bounded fixture behavior. It does not authorize production data,
installation, export, labels, larger caps, a general composed query view or an SDK
adapter. Shared blank nodes are preserved by acquisition but explicitly unsupported
by the current SDK parser; no per-graph renaming is performed.

Cargo may add its untracked `.cargo-ok` marker to the dependency checkout. The
producer's build stamp then honestly reports `producer_git_dirty=true`. Preserve
that receipt value and marker; record the actual checkout status and tracked
source hashes rather than overriding the stamp or claiming a clean build.
