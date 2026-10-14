# Due-date CLI transcript

The same dry-run `create 'example work' --priority 1 --json` command has no due
date with the previously installed CLI/unconfigured project. With the candidate
and `priority_days=[1,3,14,30,90]`, the result has a date three calendar days
from its creation time. Both runs use an isolated fixture store and an example
project graph; no operational board is written.

[Actual transcript](transcript.json). This is a readable transcript, not a video;
no terminal recording executable was available. The configuration change is
explicit, so this is candidate configuration behavior, not installed acceptance.
Source base ff464372c8d92cfdcfb954e234bdd1f7c52a845e; candidate content includes
configured creation, reasoned redating and atomic reviewed backfill. The CLI
round-trip and core controls reproduce explicit-date preservation, guarded
redating, stale-plan refusal, foreign-graph refusal and atomic successful apply.

Repeat with `python3 docs/probes/pr-demos/due-dates/reproduce.py --before /path/to/old-sd --candidate /path/to/candidate-sd`.
