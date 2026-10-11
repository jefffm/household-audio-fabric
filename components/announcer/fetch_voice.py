"""Build-time only, immutable model/config with SHA-256 verification."""

import hashlib
import json
import sys
import urllib.request
from pathlib import Path

lock = json.loads(Path(__file__).with_name("voices.lock.json").read_text())
destination = Path(sys.argv[1])
destination.mkdir(parents=True, exist_ok=True)
for name, checksum in lock["files"].items():
    path = destination / name
    digest = hashlib.sha256()
    with urllib.request.urlopen(lock["base_url"] + name, timeout=60) as source, path.open("wb") as target:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
            target.write(chunk)
    if digest.hexdigest() != checksum:
        path.unlink()
        raise SystemExit("voice_checksum_mismatch")
