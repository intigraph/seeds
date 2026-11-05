# Shared-applier board demonstration

Run `bash docs/probes/stream-board/run.sh` from the repository root. It reuses
the synthetic source/applier fixture from `tests/stream_projection.rs`.

Watch the same local replica refuse a heartbeat-only read, return `before`
after a signed challenge, return `after` following a replicated title and
assignee update, then refuse reads after disconnect. The millisecond clock
values are fixture inputs, not a measured production freshness result.

The source base is `4dc2f29e`; the demonstrated candidate is `e42a949b`.
Both recorded states use the candidate; the base has no `StreamBoard` API.
This is not a baseline performance comparison.
The recording is [demo.cast](demo.cast), an asciicast v2 terminal recording.
Download and replay with `asciinema play demo.cast`; inline playback is not
assumed. Setup uses temporary local stores and a temporary demo crate. It
does not acquire a production bootstrap, contact a live graph service or
switch a CLI or Telegram reader. The exact Git dependency and its registry
release prerequisite remain separate from installed acceptance.
