#!/usr/bin/env python3
"""Feed raw 48 kHz, signed 32-bit little-endian stereo PCM to MA-Test-PCM."""
import argparse
import math
import socket
import struct
import time

parser = argparse.ArgumentParser()
parser.add_argument("--host", default="127.0.0.1")
parser.add_argument("--port", type=int, default=14954)
parser.add_argument("--seconds", type=float, default=2.0)
parser.add_argument("--frequency", type=float, default=440.0)
args = parser.parse_args()
rate = 48000
frames_per_chunk = 960  # 20 ms
amplitude = (1 << 29)
end = time.monotonic() + args.seconds
frame = 0
with socket.create_connection((args.host, args.port), timeout=5) as sock:
    while time.monotonic() < end:
        chunk = bytearray()
        for _ in range(frames_per_chunk):
            sample = int(amplitude * math.sin(2 * math.pi * args.frequency * frame / rate))
            chunk.extend(struct.pack("<ii", sample, sample))
            frame += 1
        sock.sendall(chunk)
        time.sleep(frames_per_chunk / rate)
