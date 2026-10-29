# Create work, then read the ready queue

This 20-second recording runs native sd 0.8.0 in a new, isolated local Quipu store. Every command names the public fixture graph and actor; the local fixture configuration sets the demo prefix. It creates one work item, reads the ready queue, then reports counts. No server, sync or production tracker is used.

Start with an absent workspace: reusing demo.db changes the count. This public example is separate from any configured team tracker.

From the repository root, prepare a new, absent workspace:

```sh
mkdir -p /tmp/seeds-demo/.seeds
cp docs/demos/terminal/fixture/config.toml /tmp/seeds-demo/.seeds/config.toml
touch /tmp/seeds-demo/.terminal-demo-sandbox
python3 docs/demos/terminal/render.py docs/demos/terminal/demo.tape.json \
  --workspace /tmp/seeds-demo --tool sd="$(command -v sd)" \
  --output /tmp/seeds-render
```

The manifest pins the executable and every source input. Deliberate reading holds differ from execution times. The renderer supplies a private HOME/XDG environment and rejects private paths, hosts and credentials before publication. Read [the complete transcript](terminal/transcript.txt) for a static alternative.

The recorder is unchanged from Tambo’s canonical `stack-terminal-24` harness at `b0765b5d8f16a6a26acb3595552f17826d965b44`, SHA-256 `bebbe000565314f18a58731f2a1a4690577db127b3673fa10a45ce74eef0f23c`.
