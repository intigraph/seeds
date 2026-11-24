# RemoteBackend host read budget (source candidate)

`ST_QUIPU_READS_PER_MINUTE=0` remains the default. An enabled budget uses the
same private `shantytown/read-budget-v1` rolling-minute ledger as Shantytown's
Python reads. RemoteBackend conservatively charges every request, including
health, bootstrap, retries and writes. This is resource admission, never write
authority. An exhausted or unsafe ledger refuses before network dispatch and
must remain distinguishable from server unavailability. Charges survive failed
requests. No network request runs under the ledger lock.

Enabled callers disable connection pooling (preventing uncharged internal socket
retries) and refuse redirects, unsupported origin spellings and non-Unix
platforms rather than claiming coverage they cannot enforce. Independent native
transfer and read-stream clients are outside this source candidate. Do not
activate a fleet budget based on this candidate until every relevant caller's
coverage, installed binary and shared-ledger behavior are qualified.

The real-binary probe is `SEEDS_TEST_SD=/absolute/qualified/sd python3
scripts/test-remote-read-budget.py`. It uses only a private loopback HTTP fixture
and disposable HOME/cache/config, with no credentials or production graph.
The installed old binary makes two requests (`/health`, `/query`) with budget1,
and still makes both with a Python-format exhausted ledger. Its default-off
control sees both requests; `where` is an independently observed zero-HTTP
location proof. These correct the earlier inference that `where` itself spends
remote requests. Concurrent-child, retry, redirect and corrupt-ledger controls
must pass against the candidate binary before any repair claim.

The first numeric-budget candidate built successfully: eight SDK probes and
97 focused remote tests passed (two explicitly artifact-gated tests ignored).
A later invalid-byte environment probe reproduced a fail-open encoding fallback.
The corrected candidate rejects that encoding. A fresh serial qualification
passed all nine real SDK probes and 97 focused remote tests, with the same two
explicitly artifact-gated tests ignored. The invalid-byte probe first failed
against the previously qualified binary and then passed against the correction.
No installed acceptance or budget activation has been claimed.
