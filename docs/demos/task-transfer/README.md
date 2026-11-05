# GitHub transfer demonstration

The installed `sd 0.11.1` rejects the transfer's selection flag with exit 2
([actual before transcript](before.txt)). Its exact build commit is unavailable;
this is a released-binary comparison, not a reconstruction of the source base.
The source base is `fc3c497dc5359b35ecee7d6a29c27e8bf1c02e60`.

Implementation `5df216f7345d4a69820aef9639abd87dc1cedc88` previews two issue
creations, exports them, then replays both as unchanged. Watch the same-project
import reuse the original IDs, the different-project import replay unchanged,
and explicit conflicts and refusals preserve the relevant controls. The recording
comes from the actual CLI against a loopback HTTP fixture with an inert credential;
it does not show an installed release or a live account.

[Output recording](output.log) and [classic timing](timing.log) are retained with
this change. Download both and replay with util-linux:

```sh
scriptreplay --log-out output.log --log-timing timing.log
```

This is a terminal recording, not an inline video. `output.log` is also the readable
transcript. Regenerate from the repository root after building the native binary:

```sh
TRANSFER_SD=target/debug/sd TRANSFER_DEMO=1 python3 tests/task_transfer.py
```

The test's default output remains a single result line. Recording mode prints
actual selected invocations and response summaries; it never prints credentials.
The later artifact commit changes documentation and the recording only; its
implementation is identical to the demonstrated implementation commit.
