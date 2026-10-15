# Reactivation clears the previous hold

The recording shows the same status update against two disposable local stores.
Before, the resumed item retains its deadline and active resume marker. After,
the deadline is null and the marker becomes historical, preserving its evidence.
The second update is a positive control: an explicit new deadline survives in
both binaries. This demonstrates CLI candidate behavior, not an installed service.

Base source: `b244b83f9c82b20f4026cfe8e6f16df6409982f7`. Candidate engine blob:
`544e3db2cec4dc420c91d36b72a9f380dfac5dfa` (the implementation in this change).

Recorded executable SHA-256:

- Before: `7685bc1f1cf69c381c7b74abb6320c4e5df8b0973da37496ae4777ec08a4a36e`
- After: `17d452001ee908c3c2e1dadf37b088ab3c8697dab8337e49e19ddd5952ecac90`

Build the two revisions in separate worktrees, then run the same reproducer:

```sh
python3 docs/demos/reactivation/reproduce.py /path/to/base/sd /path/to/candidate/sd
```

[Output transcript](output.txt) and [timing](timing.txt) retain the actual
1.4-second session. Download both to replay with util-linux:

```sh
scriptreplay --log-out output.txt --log-timing timing.txt
```

Playback is a terminal recording, not an inline video. Fixtures use temporary
local stores and do not contact a live board. The core regression suite also
checks past deadlines, historical reads, explicit new holds, metadata-only edits,
claim, undefer, and the refusal to replace unrelated notes without force.
