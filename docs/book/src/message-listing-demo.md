# Complete message reads

The same authorized, read-only message selection fails with installed sd 0.19.2
and succeeds with candidate source `006261c255ed4cb1735cb80313a9f7b46f153fc7`
(based on `243a449e4b3285196d6878abbfab6ec95e3c815f`). Watch exit 1 / FAILED
become exit 0 with all required fields present. The selected message was valid;
the old weekly listing stopped partway through it at the server result cap.

[Recorded output](demos/messages/output.log), [timing](demos/messages/timing.log)
and [probe](demos/messages/probe.py) are small retained repository artifacts.
Download the output and timing files together and replay with util-linux:

```sh
scriptreplay --log-out output.log --log-timing timing.log
```

Playback was checked; inline playback is not promised. The recording emits only
status and field-presence summaries. Message identity, body, recipient, endpoint
and credentials are deliberately not recorded. Recorder metadata is sanitized.
It proves candidate behavior against the same live read, not installation,
read-mark writes or end-to-end inbox-pointer closure.

To repeat, configure the normal permitted mailbox connection and export
`BASE_SD`, `CANDIDATE_SD` and `DEMO_MESSAGE_ID` for binaries and a message you are
authorized to read; then run `python3 probe.py`. No read mark is written.
The normal JSON contract is unchanged.

The source regression separately covers over 10,000 facts, complete messages
after the old cutoff, named malformed records alongside good mail, and refusal
of truncated or non-progressing subject pages. A live traversal is not an atomic
mailbox snapshot.
