#!/usr/bin/env python3
"""Local deterministic controls only; no producer, network or acquisition trial."""
import os
import subprocess
import sys

print("Library-only admission and create-only persistence controls", flush=True)
run = subprocess.run(
    [os.environ.get("CARGO", "cargo"), "test", "--locked", "--lib",
     "native::remote::acquisition::tests"],
    capture_output=True, text=True, check=False,
)
for line in run.stdout.splitlines():
    if line.startswith(("running ", "test native::remote::acquisition::tests::", "test result:")):
        print(line)
if run.returncode:
    print(f"Cargo controls failed (exit {run.returncode}); inspect local build diagnostics.")
sys.exit(run.returncode)
