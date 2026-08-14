#!/usr/bin/env python3
import json
import os
import pathlib
import struct
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "scripts"))
import rpc

URL = f"http://127.0.0.1:{os.getenv('SNAPCAST_HTTP_PORT', '11780')}/jsonrpc"
OUTPUT = pathlib.Path(os.environ["TEST_OUTPUT_DIR"]) / "decoded.pcm"
PROJECT = os.environ["COMPOSE_PROJECT_NAME"]

def status():
    return rpc.call(URL, "Server.GetStatus")["server"]

def wait_for(predicate, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    raise AssertionError(f"timed out waiting for {description}")

server = status()  # Exercises Server.GetStatus over HTTP JSON-RPC.
stream_ids = {stream["id"] for stream in server["streams"]}
assert {"idle", "AirPlay", "MA-Test-PCM"} <= stream_ids, stream_ids

group = wait_for(lambda: (status()["groups"] or [None])[0], "test Snapclient group")
result = rpc.call(URL, "Group.SetStream", {"id": group["id"], "stream_id": "MA-Test-PCM"})
assert result["stream_id"] == "MA-Test-PCM", result

inspect = json.loads(subprocess.check_output(["docker", "inspect", f"{PROJECT}-snapserver-1"], text=True))[0]
assert inspect["HostConfig"]["ReadonlyRootfs"] is True
assert inspect["Config"]["User"] == "snapserver:snapserver"
assert inspect["HostConfig"]["CapDrop"] == ["ALL"]
assert "no-new-privileges:true" in inspect["HostConfig"]["SecurityOpt"]

writer = threading.Thread(target=lambda: subprocess.run([
    sys.executable, "scripts/synthetic-pcm.py", "--seconds", "4",
    "--port", os.getenv("MA_TEST_PCM_PORT", "14954")
], check=True))
writer.start()
wait_for(lambda: next(s for s in status()["streams"] if s["id"] == "MA-Test-PCM")["status"] == "playing",
         "MA-Test-PCM to enter playing state")
writer.join()

# Snapclient's file player writes decoded raw PCM. Prove format, duration, and signal.
def complete_pcm():
    if not OUTPUT.exists():
        return False
    size = OUTPUT.stat().st_size
    return size >= 48000 * 2 * 4 * 2 and size % (2 * 4) == 0
wait_for(complete_pcm, "at least two seconds of decoded 48000:32:2 PCM")
data = OUTPUT.read_bytes()
samples = struct.iter_unpack("<i", data)
nonzero = sum(1 for (sample,) in samples if sample != 0)
assert nonzero > 48000, f"decoded output is silent: only {nonzero} nonzero samples"
print(f"PASS: JSON-RPC switch and {len(data)} bytes of non-silent decoded 48000:32:2 PCM")
