# Read attribution demonstration

The same bounded loopback fixture uses the real SDK subprocess and native plate
HTTP reader, without production credentials. Each SDK case makes nine requests
and returns missing-item exit 3; the plate ASK-false case makes one request and
returns the same result.

| Case | Before | Source result |
|---|---|---|
| Explicit SDK caller/task/session | All three sent | All three preserved |
| Known harness crew/task | Both sent | Both preserved |
| Direct SDK, missing caller | Agent names executable `seeds` | Agent omitted; source `seeds`, finite endpoint operation |
| Scoped plate with supplied task/session | Caller sent, task/session omitted | Caller, task and session sent; source/operation separate |

The source result passes 100 SDK library controls and two real built-CLI transport
tests, including missing/stale-context negatives. Writer provenance and
signed/bearer transport retain their prior contract. No request-count reduction,
installed change or fleet query-rate acceptance is claimed by this demonstration.

An isolated process regression also preserves the writer snapshot at the first
request: a read under caller `before`, followed by a child-only environment change
to `after`, still builds its write with caller `before`. The read attribution split
initially delayed this initialization; the correction restores the legacy timing.
No network traffic or parent environment change is needed for that control.
