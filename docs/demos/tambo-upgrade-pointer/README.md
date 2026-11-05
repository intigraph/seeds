# Current installer pointer

Run `sd upgrade`. The command names Tambo and its update command, while
retaining exit 21 to signal that the operation belongs to another tool.
The CLI test checks the same contract.

This is an actual candidate CLI recording, not an installed-service claim.
Replay it with `scriptreplay --log-timing timing.txt --log-out output.txt`.
The upgrade command only returns a pointer; it does not run an installer.
