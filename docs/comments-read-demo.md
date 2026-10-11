# Current comment reads stay scoped

Trigger: list the comments of one item through an indexed backend. The base
`0d44421bb96f97a036936a5fa29cfa0c6f115cdc` route reads a whole snapshot.
The candidate route asks for that item's existing scoped snapshot instead.

The [terminal recording](demos/comments-read.txt) and
[timing file](demos/comments-read.timing) show the same three regression probes:
base route **2 failed / 1 passed**, scoped route **3 passed**. The first probe
checks comment content/order, unrelated items and missing IDs. The second
checks historical pins and an unindexed backend. The third propagates an
unavailable scoped read without falling back to a whole snapshot.

Replay after downloading both files:

```sh
scriptreplay --log-out comments-read.txt --log-timing comments-read.timing
```

Reproduce from the PR's exact head in a clean disposable checkout:

```sh
bash docs/demos/comments-read.sh
```

The script temporarily restores only the base `comment_list` route, runs the
fixture, and restores the candidate even on failure. This is a library fixture
recording, not an installed-service or fleet-rate acceptance claim. The separate
real-server parity corpus compares all item IDs, including ephemeral comments,
against whole-snapshot output. The exact demonstrated candidate revision is
bound in the PR caption. The small recording is retained with this document;
playback requires downloading it and using `scriptreplay`.
