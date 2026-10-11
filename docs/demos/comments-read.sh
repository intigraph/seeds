#!/usr/bin/env bash
set -euo pipefail
saved=$(mktemp)
log=$(mktemp)
cp src/engine.rs "$saved"
trap 'cp "$saved" src/engine.rs; rm -f "$saved" "$log"' EXIT
python3 - <<'PY'
from pathlib import Path
p=Path('src/engine.rs');s=p.read_text()
start=s.index('pub fn comment_list(');end=s.index('\n}',start)
chunk=s[start:end]
a=chunk.index('    // Exact-ID');z=chunk.index('    snap.get(id)?;')
chunk=chunk[:a]+'    let snap = b.snapshot(at)?;\n'+chunk[z:]
p.write_text(s[:start]+chunk+s[end:])
PY
printf '%s\n' 'BEFORE: base 0d44421 comment_list route + regression fixture' 'cargo test --test core comment_list_'
set +e
cargo test --test core comment_list_ > "$log" 2>&1
rc=$?
set -e
sed -n -E '/^running |^test |^test result:|assertion|left:|right:|current comments read/p' "$log"
test "$rc" -eq 101
cp "$saved" src/engine.rs
printf '%s\n' 'AFTER: scoped current-item route; same fixture and command' 'cargo test --test core comment_list_'
cargo test --test core comment_list_ > "$log" 2>&1
sed -n -E '/^running |^test |^test result:/p' "$log"
