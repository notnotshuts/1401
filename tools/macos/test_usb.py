#!/usr/bin/env python3
"""Actions-only native USB integration using an owned disposable disk image."""
import json
import os
from pathlib import Path
import subprocess
import sys

if sys.platform != "darwin" or os.environ.get("GITHUB_ACTIONS") != "true":
    raise SystemExit("This integration runner is only for macOS GitHub Actions")
root = Path(__file__).resolve().parents[2]
build = subprocess.run(["cargo", "test", "-p", "ferrite-core", "--locked", "--no-run", "--message-format=json"], cwd=root, text=True, capture_output=True, check=True)
executables = []
for line in build.stdout.splitlines():
    entry = json.loads(line)
    if entry.get("reason") == "compiler-artifact" and entry.get("profile", {}).get("test") and entry.get("executable"):
        executables.append(entry["executable"])
if len(executables) != 1:
    raise SystemExit("Expected exactly the ferrite-core unit-test executable")
subprocess.run(["sudo", "/usr/bin/env", "GITHUB_ACTIONS=true", executables[0], "macos_disk::tests::virtual_disk_claim_and_write", "--exact", "--ignored", "--nocapture"], cwd=root, check=True)
