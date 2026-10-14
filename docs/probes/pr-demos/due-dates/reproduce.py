"""Compare an old unconfigured CLI with the configured candidate in private dry runs."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--before", required=True, help="old CLI executable")
parser.add_argument("--candidate", required=True, help="candidate CLI executable")
args = parser.parse_args()
with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / ".seeds").mkdir()
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("SEEDS_", "QUIPU_", "BR_"))}
    env.update(HOME=directory, XDG_CONFIG_HOME=directory + "/config")
    def run(binary):
        result = subprocess.run([str(Path(binary).resolve()), "--store", str(root / "fixture.db"),
                                 "--graph", "https://example.org/project/demo", "create",
                                 "example work", "--priority", "1", "--dry-run", "--json"],
                                cwd=directory, env=env, text=True, capture_output=True, check=True)
        value = json.loads(result.stdout)
        return dict(priority=value["priority"], due_at=value["due_at"],
                    created_at=value["created_at"], dry_run=True)
    before = run(args.before)
    (root / ".seeds/config.toml").write_text("[due_dates]\npriority_days=[1,3,14,30,90]\n")
    after = run(args.candidate)
    print(json.dumps(dict(before_unconfigured_installed_cli=before,
                         after_configured_candidate=after,
                         limit="Isolated dry-run fixture; configuration enabled only for candidate. No installed acceptance or board backfill."), indent=2))
