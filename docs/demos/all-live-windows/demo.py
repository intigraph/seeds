"""Read an old message through the actual native CLI, in a disposable local store."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--sd", required=True)
parser.add_argument("--quipu", required=True)
args = parser.parse_args()
sd, qp = str(Path(args.sd).resolve()), str(Path(args.quipu).resolve())
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    (root / "home").mkdir()
    env = {"PATH": "/usr/bin:/bin", "HOME": str(root / "home"),
           "XDG_CONFIG_HOME": str(root / "home/config"), "SEEDS_ACTOR": "reader"}
    graph = "https://example.invalid/mail-demo"
    (root / "message.ttl").write_text('''@prefix schema: <https://schema.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<https://seeds.local/message/msg-old-demo> a schema:Message ;
 schema:identifier "msg-old-demo" ; schema:recipient <https://seeds.local/principal/reader> ;
 schema:dateSent "2000-01-03T10:00:00Z"^^xsd:dateTime ; schema:text "old live mail" .
''')
    seed = subprocess.run([qp, "knot", "message.ttl", "--db", "mail.db", "--graph",
                           graph + "/messages/2000-W01"], cwd=root, env=env,
                          capture_output=True, text=True, check=True)
    print(json.dumps({"fixture": "one native message dated 2000-01-03 in a registered local window",
                      "sd_sha256": hashlib.sha256(Path(sd).read_bytes()).hexdigest(),
                      "quipu_sha256": hashlib.sha256(Path(qp).read_bytes()).hexdigest()}))
    rows = []
    for operation in [["list", "--to", "reader", "--unread"],
                      ["list", "--to", "reader", "--unread", "--all-windows"],
                      ["read", "msg-old-demo", "--all-windows"],
                      ["count", "--to", "reader", "--unread", "--all-windows"]]:
        out = subprocess.run([sd, "--store", "mail.db", "--graph", graph, "msg",
                              *operation, "--json"], cwd=root, env=env,
                             capture_output=True, text=True, check=True)
        value = json.loads(out.stdout)
        print(json.dumps({"command": ["sd", "msg", *operation], "exit": out.returncode, "response": value}))
        rows.append(value)
    assert rows[0] == [] and rows[1][0]["id"] == "msg-old-demo"
    assert rows[2][0]["marked"] is True and rows[3]["count"] == 0
