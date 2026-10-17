"""Record only synthetic loopback controls; no deployment or real credentials."""
import os
import subprocess
import sys

for label, key, legacy in [
    ("BEFORE (installed 0.4.2; not a build of the PR base)", "CREDENTIAL_BEFORE_SD", True),
    ("AFTER (candidate built from the proposal)", "CREDENTIAL_AFTER_SD", False),
]:
    env = dict(os.environ, CREDENTIAL_SD=os.environ[key])
    if legacy:
        env["CREDENTIAL_EXPECT_LEGACY"] = "1"
    else:
        env.pop("CREDENTIAL_EXPECT_LEGACY", None)
    print(label, flush=True)
    print("$ python3 tests/credential_descriptor.py", flush=True)
    subprocess.run([sys.executable, "tests/credential_descriptor.py"], env=env, check=True)
