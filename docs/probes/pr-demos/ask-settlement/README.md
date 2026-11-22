# Atomic Ask settlement library demonstration

Run `cargo run --example ask_settlement_demo` from this source checkout.
The example uses an in-memory Quipu backend and a **synthetic** admitted native
port. It verifies transaction behavior, not WebAuthn, production admission,
durable on-disk custody or an installed service.

On base `70f502d22e8e1ba0cfccda43bcf15550a2692773` the new API is absent;
there is no equivalent settlement call. On the PR head containing this fixture,
the same open Ask and linked work begin with no ruling. An absent native port
refuses without a transaction. One admitted batch then closes the Ask, keeps
linked work open and adds exactly one ruling. Read-only recovery leaves the
transaction unchanged.

[Terminal recording](output.typescript) and [timing](timing.txt) are small,
retained source artifacts. Download both and replay with
`scriptreplay --log-out output.typescript --log-timing timing.txt`.
The recording is readable as a transcript; no inline video playback is claimed.
The recording runs a copied build artifact in a local scratch directory.
The accompanying tests independently cover a race on either item, a lost commit
response and prior custody that forbids another send.
