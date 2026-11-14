# Configured signing admission

A launcher with a configured signing session previously had only the bearer-only
admission probe, which refuses that configuration. The candidate keeps that
refusal and adds a distinct configuration-only proof that no credential provider
is required for the existing signed native path. Watch `authenticated: false`:
this is preparation, not enrollment proof or permission to retry a refused write.

The [recording](output.txt), [timing](timing.txt) and [reproducer](reproduce.py)
use only synthetic configuration with a missing key and an example destination.
Both commands run on the candidate; the first demonstrates the retained legacy
behavior, not a separate base-binary build. Base source is
`c95eb1d`; the PR identifies the implementation revision. No create is executed.
The signed create, original key/audience and actor controls are separately
exercised by `tests/actor_principal.rs` against a loopback fixture.

Replay with `scriptreplay --log-out output.txt --log-timing timing.txt` or run
`python3 reproduce.py /absolute/path/to/sd`. The small recording and caption
are retained with this probe directory under the repository's existing demo
convention. No deployed behavior is claimed.
