# Isolated acquisition verifier

This is a one-shot, unpublished helper workspace. It does not serve HTTP,
open a store, obtain credentials or acquire data. A separate lockfile pins
the reference producer without changing the SDK's dependency graph.

Build with `cargo build --locked --release --manifest-path
tools/acquisition-verifier/Cargo.toml`. The caller must record the executable
SHA-256, this workspace's lockfile hash and exact source pin before use.
Never discover a verifier on PATH or use a downloaded binary without that
out-of-band binding.

Input is a JSON header line (`schema: seeds-acquisition-input/v1`,
`request_bytes`, `wire_bytes`), followed by exactly those request and original
wire bytes. No JSON re-encoding or outer-whitespace trimming occurs. The helper
invokes the pinned producer's `Payload::verify`, then
returns the exact request and wire byte digests with a protocol/build ID.
Decoding a receipt is not a verification substitute. Invalid input exits 1
without printing its contents. No alternate verifier is selected on failure.

Linux limits are fixed: 1 GiB address space, two seconds CPU, 4 KiB output,
and 268,500,992 input bytes. Other platforms refuse. The SDK also enforces a
total wall-clock deadline and executes a private copy of the hash-checked
binary. Resource refusal does not authorize raising a cap.

The pinned reference is historical. Compiling it does not establish current
server compatibility, full-size capacity or production eligibility. No
acquisition trial or activation is part of this source addition.
