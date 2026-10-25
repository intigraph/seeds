#!/usr/bin/env bash
set -euo pipefail
root=$(git rev-parse --show-toplevel)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
python3 - "$root" "$fixture" <<'PY'
import json
import pathlib
import sys
root, fixture = map(pathlib.Path, sys.argv[1:])
manifest = f'''[package]
name = "stream-board-demo"
version = "0.0.0"
edition = "2021"
[dependencies]
seeds = {{ package = "seeds-ai", path = {json.dumps(str(root))} }}
quipu = {{ package = "quipu-ai", git = "https://github.com/intigraph/quipu", rev = "fec93ae63adcdf49e7bf1ce5e6b060f9650bb5e9", default-features = false }}
ring = "0.17"
tempfile = "3"
serde_json = "1"
'''
(fixture / 'Cargo.toml').write_text(manifest)
# Reuse the exact synthetic source/applier setup exercised by the tests.
setup = (root / 'tests/stream_projection.rs').read_text().split('#[test]', 1)[0]
setup = '\n'.join(line for line in setup.splitlines()
                  if not line.startswith(('#!', '//!')))
setup = setup.replace('use seeds::backend::{Backend, Ctx};',
                      'use seeds::backend::Ctx;')
(fixture / 'src').mkdir()
(fixture / 'src/main.rs').write_text(setup + r'''
fn main() {
    let mut t = Tree::new();
    let heartbeat = t.session.heartbeat();
    t.feed(vec![heartbeat], 2);
    let heartbeat_only = t.replica.list(&ListReq::default(), 2, 5000).is_ok();
    println!("heartbeat only: board admitted={heartbeat_only}");
    assert!(!heartbeat_only);
    t.prove(1, 3, 4);
    let before = t.replica.list(&ListReq::default(), 4, 5000).unwrap();
    println!("signed challenge: board title={:?}, bound={:?}",
             before.value.issues[0].title, before.bounded_freshness_ms);
    engine::update(&mut t.source, &ctx(), std::slice::from_ref(&t.id),
        &UpdateReq { title: Some("after".into()),
                     assignee: Some("reader".into()), ..Default::default() }).unwrap();
    t.prove(2, 5, 6);
    let mut plate = ListReq::default();
    plate.filter.assignee = Some("reader".into());
    let after = t.replica.list(&plate, 6, 5000).unwrap();
    println!("replicated commit: plate title={:?}, assignee={:?}",
             after.value.issues[0].title, after.value.issues[0].assignee);
    assert_eq!(after.value.issues[0].title, "after");
    assert_eq!(t.replica.ready(&ctx(), &ReadyReq::default(), 6, 5000)
        .unwrap().value.issues[0].id, t.id);
    t.replica.disconnect();
    let disconnected = t.replica.list(&ListReq::default(), 7, 5000).is_ok();
    println!("disconnect: board admitted={disconnected}");
    assert!(!disconnected);
    println!("Synthetic library demonstration complete; no live reader switched.");
}
''')
PY
cargo run --quiet --manifest-path "$fixture/Cargo.toml"
