# Source-only acquisition API demonstration

Before this change the pinned verifier existed only in synthetic integration
fixtures. After this change an explicit library-only client negotiates the
contract and verifies exact wire bytes before create-only artifact publication.
The CLI remains unwired; remote historical export still refuses.

Run `python3 docs/demos/acquisition-api/reproduce.py`. The recording contains
local request/pin/budget and raw-byte/create-only persistence controls. It runs
no server, performs no acquisition and establishes no current producer or
production eligibility. Timing is a recording aid, not a performance claim.

Replay: `scriptreplay --log-out docs/demos/acquisition-api/output.log
--log-timing docs/demos/acquisition-api/timing.log`.
