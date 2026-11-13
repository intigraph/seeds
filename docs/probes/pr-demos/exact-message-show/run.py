"""Private loopback fixture replay: released CLI versus candidate, no credentials."""
import importlib.util
import os
import pathlib

root = pathlib.Path(__file__).resolve().parents[4]
spec = importlib.util.spec_from_file_location("wire", root / "tests/message_show_wire.py")
wire = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wire)
print("Synthetic private mailbox: requested legacy target is valid; neighbour is malformed/capped.")
print("Released baseline (sd 0.20.1):")
os.environ["MESSAGE_SD"] = os.environ["BASELINE_SD"]
wire.run("good-with-bad-neighbour", baseline=True, verbose=True)
print("Candidate source:")
os.environ["MESSAGE_SD"] = os.environ["CANDIDATE_SD"]
wire.run("good-with-bad-neighbour", verbose=True)
print("Candidate malformed-target negative control:")
wire.run("malformed", verbose=True)
