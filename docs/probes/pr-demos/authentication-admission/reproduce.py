"""Local admission demo: no key/token exists and no network call is made."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile

binary = str(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else os.environ["AUTH_DEMO_SD"]).resolve())
with tempfile.TemporaryDirectory() as tmp:
    root = pathlib.Path(tmp)
    config = root / ".config/seeds"
    config.mkdir(parents=True)
    (config / "config.toml").write_text(
        '[quipu]\nsigning_key_file="unread-key"\n'
        'signing_session="example-session"\nsigning_introducer="example-lead"\n'
    )
    env = {"PATH": os.environ["PATH"], "HOME": tmp, "XDG_CONFIG_HOME": str(root / ".config")}
    common = [binary, "--quipu", "https://example.org", "--actor", "example-worker", "create", "Example operation"]
    for flag, expected in [("--credential-admission", 6), ("--authentication-admission", 0)]:
        print(f"$ sd create 'Example operation' --actor example-worker {flag}", flush=True)
        result = subprocess.run(common + [flag], cwd=tmp, env=env, capture_output=True, text=True, timeout=10)
        print(result.stdout.strip(), flush=True)
        print(f"exit {result.returncode}", flush=True)
        assert result.returncode == expected
        if expected == 0:
            proof = json.loads(result.stdout)
            assert proof["provider_required"] is False and proof["authenticated"] is False
    assert not (root / "unread-key").exists() and not (root / ".seeds").exists()
    print("No operation executed; signing key absent; original identity configuration retained.", flush=True)
