"""Read-only comparison; record no message body, identity, endpoint or credential."""
import json, os, subprocess

def run(label, binary):
    argv = [binary, "msg", "show", os.environ["DEMO_MESSAGE_ID"], "--json"]
    result = subprocess.run(argv, capture_output=True, text=True)
    value = json.loads(result.stdout)
    print(label, "version:", subprocess.check_output([binary, "--version"], text=True).strip())
    print("command: sd msg show <selected-message> --json (identity redacted)")
    print("exit:", result.returncode)
    if result.returncode:
        print("error code:", value["error"]["code"])
    else:
        assert value["id"] == os.environ["DEMO_MESSAGE_ID"]
        print("selected message returned:", True)
        complete = all(value.get(k) for k in ["id", "to", "sent_at", "text"])
        print("required fields present:", complete)
        assert complete
    return result.returncode

assert run("BASE installed", os.environ["BASE_SD"]) != 0
assert run("CANDIDATE source", os.environ["CANDIDATE_SD"]) == 0
print("PASS: same read-only selection failed before and returned a complete message after.")
