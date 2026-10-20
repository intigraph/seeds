# Reproducible release API comparison

The release workflow still uses release-plz 0.3.169 and cargo-semver-checks
0.50.0. Its API baseline is still the source of the previously released Seeds
version. No semver check, feature, lint, or failure is disabled.

The original release jobs for changes 166 and 167 failed before comparison:
`shacl 0.3.16` constructed a literal without the `direction` field required by
`rudof_rdf 0.3.29`. The logs label that arm **current**, even though both the
current crate and released crate were version 0.6.1. The committed lockfiles
selected `rudof_rdf 0.3.24`. Describing this as a baseline-only failure was
incorrect.

Cargo-semver-checks 0.50 creates a placeholder project that depends on the
source crate and runs `cargo update` there. A path dependency's own lockfile
does not constrain that new project. The release job therefore compiled a
different dependency resolution from the checked-in one.

`scripts/ci/cargo-semver-checks-locked.py` adapts release-plz's existing
`--manifest-path` / `--baseline-root` invocation. It:

1. Verifies the baseline's manifest, lock and complete `src` file set against
   the release commit resolved from `seeds-ai-v<version>`. Missing or changed provenance
   refuses; it does not fall back to another API baseline.
2. Generates rustdoc for both original source trees using their own lockfiles
   and `cargo rustdoc --locked --lib --all-features`. It retains the private and
   hidden-item flags used by cargo-semver-checks. The currently supported feature
   set is explicitly checked; adding a feature requires reviewing coverage.
3. Passes the resulting JSON to cargo-semver-checks' supported
   `--current-rustdoc` and `--baseline-rustdoc` options. Its stdout and exact exit
   status propagate: incompatible API is 100, a build/tool error remains an error.

The historical source is not patched or republished. No crate, manifest or
dependency lock is changed by this repair. The workflow installs the same tools
separately to insert the adapter, then retains the upstream action's
`release-pr` / `release` commands, permissions, and JSON output contract.
Downstream unlocked installation still follows the dependency constraints in
the manifest; this repair is scoped to reproducible release API analysis.

## Validation and demonstration

An isolated dependency-wrapper reproduction using the original 167 source
resolved 0.3.29 and reproduced E0063. With locked rustdoc generation, that exact
source compiled and comparison completed with exit 100: real added struct
fields and an enum variant were reported instead of a compiler error.

The actual 0.6.1 tag compared with itself returns 0. A private copy of the same
release with `pub mod rank` changed to private returns 100. This negative
control proves that compiling rustdoc does not silently accept incompatible API.

Repeat the real API controls with Rust 1.98.1 and the pinned tool installed:

```sh
python3 scripts/ci/prove-locked-semver.py --real "$(command -v cargo-semver-checks)"
python3 -m unittest discover -s scripts/ci/tests -p test_locked_semver.py
```

This is source-fixture evidence, not an installed release or a successful
publisher run. Reverting the adapter and workflow changes restores the prior
release analysis route; retain the original failure rather than calling an
already-up-to-date release run proof that the failed comparison compiled.

A small replayable terminal recording of the real JSON comparison calls is
retained in `docs/demos/locked-semver/` (caption, output log and timing file).
It replays results from the locked builds, not the long compilation. The
recorder's private driver path is redacted; output and timing are retained.

Sources: [release-plz's pinned comparison invocation](https://github.com/release-plz/release-plz/blob/release-plz-v0.3.169/crates/release_plz_core/src/semver_check.rs),
[cargo-semver-checks' placeholder resolution](https://github.com/obi1kenobi/cargo-semver-checks/blob/v0.50.0/src/data_generation/generate.rs),
and [its rustdoc inputs](https://github.com/obi1kenobi/cargo-semver-checks/blob/v0.50.0/src/main.rs).
