# Shared-key actor CLI recording

One synthetic host key serves `worker-a` and `worker-b`. The unreleased
compatibility candidate refuses the mismatch in explicit `enforce` mode, then
accepts both actors with the default `observe` policy. The actual observation
file records both actors, `host-principal`, their commands/times and total 2.
The HTTP fixture receives each declared actor unchanged.

[Raw recording](output.txt.gz), [readable transcript](transcript.txt), [timing](timing.txt),
[repeatable fixture](reproduce.py). These small text artifacts use the existing
versioned PR-demo directory and stay with the source. The gzip preserves the
original recording bytes and timing offsets; only the readable copy normalizes
CRLF to LF. No media host is required.
This is a terminal recording, not an inline video. Replay downloaded files with:

```sh
gzip -dc output.txt.gz > output.txt
scriptreplay --log-out output.txt --log-timing timing.txt
```

The base source is `22a3861` (0.7.1). The recording executes the **candidate**
with enforce as the refusal control; it does **not** execute that base binary.
The exact candidate binary SHA256 is printed in the recording. The Rust product
sources match this PR; adding these documentation artifacts does not rebuild the
binary. The PR body identifies the reviewed source head. The fixture emulates
the HTTP protocol and accepts synthetic updates; it does not validate the
server's cryptography, read committed board data or prove installed acceptance.

Build the candidate and repeat:

```sh
cargo build --bin sd
python3 docs/probes/pr-demos/actor-principal/reproduce.py target/debug/sd
```

Use Cargo's configured target directory when build output is redirected.

The script creates and removes its own temporary home, config, state and a
public synthetic key. It binds only loopback. The alternate IPv4 URL spelling
`127.1` reaches that listener through normal URL canonicalization, while taking
the client's conservative non-exempt HTTP transport branch. No DNS, real board,
installed configuration, host allowance or live credential participates.

The stronger executable regression fixture is `tests/actor_principal.rs`:
default observe, two actors under one key, positive counter, matching/mismatching
enforce, off, corrupt/contended-counter UNKNOWN with a bounded lock wait, and
bearer refusal before any HTTP request.
