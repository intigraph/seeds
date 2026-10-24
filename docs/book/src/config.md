# Configuration

With no configuration at all, `sd` uses a **local** quipu store at
`.seeds/seeds.db` and creates it on the first write. Nothing needs a server.

## Where the graph lives

Two keys, and a store uses exactly one of them:

```toml
# .seeds/config.toml (or ~/.config/seeds/config.toml)
[quipu]
store = ".seeds/seeds.db"            # a local quipu store file (the default)
# url = "https://quipu.example.org"  # OR a shared quipu server
# token_file = "~/.config/seeds/quipu-token"   # bearer for writes: USER config only
# trusted_hosts = ["quipu.example.org"]       # USER config only: see below
# allow_plain_http_hosts = ["quipu.internal.example"]      # USER config only: see below
# signing_key_file = "~/.config/seeds/keys/<session>.key"  # signed writes: USER config only
# signing_session = "<session>"
# signing_introducer = "<who registered it>"

[project]
prefix = "sd"                        # id prefix for new seeds (default sd)
# graph = "https://seeds.local/project/sd"   # the project's named graph

[pendant]
# dir = ".seeds/pendant"             # keep the ledger in the repo (mode 1)

[sync]
# remote = "https://quipu.example.org"   # what `sd sync` exchanges with (mode 3)
```

Which combination gives which arrangement is in
[Storage modes](storage-modes.md).

- `store` is a path. Relative paths resolve against the directory that holds
  `.seeds/` (in the project file) or against the user config directory (in the
  user file). `~/` expands to your home directory.
- `url` is an `http://` or `https://` quipu server.
- A file (or the environment) that sets **both** is refused with exit 6.
  Choose one.
- `graph`, when set, must be a plain absolute IRI; anything that could break
  out of the SPARQL it is written into is refused (exit 6).
- Unknown keys are refused too, so a typo such as `stroe` fails loudly instead
  of being ignored.

## Precedence

Highest first. A layer is consulted only when every layer above it says
nothing about that setting.

| # | layer | location keys | other keys |
|---|---|---|---|
| 1 | flags | `--store <path>`, `--quipu <url>` | `--graph` |
| 2 | environment | `SEEDS_QUIPU_STORE`, `SEEDS_QUIPU_URL` | `SEEDS_GRAPH`, `SEEDS_PREFIX`, `SEEDS_ACTOR`, `SEEDS_PENDANT_DIR`, `SEEDS_SYNC_REMOTE`, `SEEDS_QUIPU_TOKEN`, `SEEDS_MAX_WRITE_BYTES`, `SEEDS_MAX_WRITE_CLAUSES` |
| 3 | project file | the nearest `.seeds/config.toml`, walking up from the current directory like git | `[project]`, `[pendant]`, `[sync]`, `token_file` |
| 4 | user file | `$XDG_CONFIG_HOME/seeds/config.toml`, else `~/.config/seeds/config.toml` | the same |
| 5 | default | a local store at `<project>/.seeds/seeds.db` | prefix `sd` |

`<project>` in the default is the directory holding the nearest `.seeds/`
(walking up), or the current directory when there is none.

The store location is **one** choice: the highest layer that names either a
`store` or a `url` decides it. A project file that says `store` wins over a
user file that says `url`; the environment wins over both; a flag wins over
everything.

## A shared server

`url` is a shared, team-wide quipu server, read and written live over HTTP
([mode 2](storage-modes.md#mode-2-a-quipu-server)). Writes send
`Authorization: Bearer <token>` when a token is configured (`SEEDS_QUIPU_TOKEN`
or `[quipu] token_file`); reads send none.

### Write provenance

Every write request to the server carries `X-Quipu-Client: seeds` and up to five
provenance headers, filled from the environment, never typed:

| header            | first of                                                                  |
|-------------------|---------------------------------------------------------------------------|
| `X-Quipu-Agent`   | `QUIPU_AGENT`; `SHANTY_AGENT` inside an agent session; `seeds`            |
| `X-Quipu-Harness` | `QUIPU_HARNESS`; `claude` (`CLAUDECODE=1`); `codex` (`CODEX_HOME`); `cli` |
| `X-Quipu-Model`   | `QUIPU_MODEL`; `SHANTY_MODEL`                                             |
| `X-Quipu-Session` | `QUIPU_SESSION`; `CLAUDE_CODE_SESSION_ID`                                 |
| `X-Quipu-Host`    | `QUIPU_HOST`; the machine's hostname                                      |

An agent session is `CLAUDECODE=1` or a set `CODEX_HOME`; outside one,
`SHANTY_AGENT` is ignored. A cron or service running `sd` names itself with
the `QUIPU_*` overrides, e.g. `QUIPU_AGENT=nightly-sync QUIPU_HARNESS=cron`.
A field that cannot be filled is omitted. Values keep only printable ASCII,
at most 128 characters, so no header can be injected.

A configured URL that cannot be reached is **exit 7**, "cannot reach quipu at
…". seeds **never** falls back to a local store when a configured server is
down, because the two would then hold different ledgers.

### Where the token may go

A cloned repository's `.seeds/config.toml` is not yours, so it is not trusted
with your token:

- `token_file`, `trusted_hosts` and `allow_plain_http_hosts` are honoured
  only in your **user** config (or `SEEDS_QUIPU_TOKEN_FILE` /
  `SEEDS_QUIPU_TOKEN`); a project file that sets any of them is refused.
- A server URL that came from a **project** file gets your token only if its
  host is in your `trusted_hosts`. A URL you chose (flag, environment, user
  file) always does.
- A token is never sent over plain `http://`, except to localhost or to a
  host your user config lists in `allow_plain_http_hosts`. That list is for a
  server on a network you trust that has no TLS. An entry matches exactly: a
  bare host (`quipu.internal.example`) admits any port on that host, `host:port` admits only
  that port, and nothing is matched by suffix.

### Signed writes: no bearer on the wire

A quipu server that accepts signed writes (`POST /knot`, `/update`, `/episode`)
does not need your bearer at all. `sd key init` makes an Ed25519 key that never
leaves your machine; its public half is registered **once** on the server by
someone else (your lead or a human, never you). After that every write carries
an `x-quipu-attestation` header: a signature over that one request (method,
path, content type, body hash) with a single-use nonce. Nothing in it can be
reused as another request. Use HTTPS or an explicitly trusted local transport.
The destination trust guard still applies to signed evidence; the plain-HTTP
transport refusal covers bearer tokens only. The server records the protected binding principal as authenticated
evidence separately from declared payload metadata.

```toml
# ~/.config/seeds/config.toml (USER config only; sd key init prints these)
[quipu]
signing_key_file = "~/.config/seeds/keys/seeds-myhost-me.key"
signing_session = "seeds-myhost-me"
signing_introducer = "<lead>"
```

- A signed write never also sends the bearer.
- A key is used only for a server you would send your token to: a URL from a
  project file needs its host in `trusted_hosts`. The signature does not name
  the server, so a server a cloned project chose could otherwise pass your
  signed write on to yours.
- `/graph/create` is signed too. seeds creates a graph only when the server
  does not already list it (`GET /graphs`). A server too old to accept a signed
  route refuses; the client does not retry that operation with a bearer.
- The key must be registered **with write granted** (`--allow-write`, or
  `quipu attest allow-write <session>` later). A key registered only to trust
  someone's shares is refused with `scope`.
- A refusal says which check failed: `skew` (fix this machine's clock),
  `unbound` (the key is not registered: `sd key show`), `scope` (registered
  without write), `revoked`/`expired`, `replay`, `badsig`.

See [sd key](verbs/key.md). To set up an agent end to end (its own user, its key,
enrollment on the Quipu host, verification and revocation), follow CABOODLE's
[agent identity and write access](https://scbrown.github.io/caboodle/agents.html#agent-identity-and-write-access)
guide.

## The project id

With no `graph` configured, a project's graph is
`https://seeds.local/project/<prefix>-<id>`, where `<id>` is a random id kept
in `.seeds/project-id`. It is created on the first write (or first use of a
server) and should be **committed**: it is what keeps two repositories that
never set a prefix from sharing one ledger on a shared server. A plain read in
a directory that never wrote creates nothing. A local store that exists
without its project id is refused rather than read as empty.

## CI

Point a job at a throwaway store with the environment, which beats any config
file in the checkout:

```bash
export SEEDS_QUIPU_STORE="$RUNNER_TEMP/seeds.db" SEEDS_ACTOR=ci
sd create "smoke" --silent
```

## Where this lives in the code

Configuration is resolved only in the native CLI (`src/native/config.rs`). The
core never reads a file or an environment variable; it is handed a storage
backend that is already open, which is what keeps it
[wasm-clean](wasm.md). The precedence rules above are unit-tested there, and
end to end in `tests/cli.rs`.


## Key-derived identity profile

`sd identity --json` sends an existing session-attestation signature to the
server's protected `/identity` endpoint. Its principal comes from the enrolled
public-key binding, not `SEEDS_ACTOR`, `USER`, or caller headers. A bearer alone
cannot answer this command. The client checks the returned key ID and session.

Set `require_identity = true` in the **user** `[quipu]` configuration only after
installing the matching server and enrolling that user's own signing key using
the existing introducer. In this profile, writes require a remote signing key;
a local or unkeyed write refuses before opening a store. Signed requests do not
carry caller-declared agent/session headers, and signature refusal never falls
back to a shared bearer.

The **user** `[quipu]` setting `actor_principal_mode` independently controls a
declared `--actor` / `SEEDS_ACTOR` differing from the verified signing principal:

Read the resolved host mode with `sd config get quipu.actor_principal_mode`.

| Mode | Mismatched declared actor |
| --- | --- |
| `off` | Keep the actor without an identity observation. |
| `observe` (default) | Keep the actor, record the mismatch, and warn. |
| `enforce` | Refuse before a destination write. |

A project cannot set this host policy. Unknown mode names refuse configuration.
In observe and enforce, an omitted actor uses the verified principal. The
signature still authenticates the key principal; a claimed actor is attribution,
not proof that the named agent owns that key. Identity/signature errors remain
errors. Neither a mismatch nor a failed observation counter refuses an observe
write.

Observe stores a cumulative `mismatches_total` and the latest 64 mismatch
attempts in `$XDG_STATE_HOME/seeds/actor-principal.json`, falling back to
`~/.local/state/seeds/actor-principal.json`. Entries contain actor, principal,
verb and time, each bounded to 128 characters. The host lock serializes counter
updates and the file is replaced atomically. These are attempted writes, not
commit receipts. Counter failure warns `UNKNOWN`, rather than claiming zero.
A deliberately mismatched canary must increment the counter before a zero
mismatch interval can establish anything. Do not turn on enforce for agents
still sharing one host key: enroll individual principals, observe a measured
interval with a daily positive control, then canary enforcement separately.

Signed requests without a bearer retain the same plain-HTTP exposure as 0.6.7.
The bearer-token refusal above is unchanged, including its explicit host
allowlist. Signatures bind the method, path, content type and body; server time
and nonce checks constrain replay, but do not encrypt traffic or bind the
destination. An interceptor can read or relay a captured request within its
validity window. Use a trusted route; moving signed traffic to HTTPS requires
the server write route and client CA trust, rather than a changed client default.

Keep the key in an owner-only regular file owned by the current effective Unix
UID. Symlinks, foreign owners, group/world permissions and oversized files are
refused through the same descriptor used to read the seed. This protects the
file boundary only when agents have **different OS users**; mode 0600 under one
shared user cannot prevent borrowing another agent's key.

Migration order: install compatible binaries; prepare distinct private user
homes; enroll each user's public binding; verify own-key identity, wrong-key,
replay and cross-user refusal; configure required identity; then activate server
key enforcement in the separately reviewed migration window. Preserve the old
configuration and public binding inventory for rollback. Never copy another
agent's private key. Do not enable strict server policy while required clients
still depend on unsigned endpoints. The signed server profile currently covers
read routes and the four audited mutation routes, without URL query strings;
unsupported mutations refuse rather than silently using bearer authentication.
