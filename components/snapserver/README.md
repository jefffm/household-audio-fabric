# Household audio Snapserver tracer bullet

An isolated, non-production Snapcast 0.35.0 package for proving the household-audio control and PCM-ingress path. It does not install any cluster resources.

## Contract

- `idle` is the silent FIFO-backed default stream.
- `AirPlay` accepts **headerless signed little-endian PCM** over TCP server mode on `14953`, fixed at `48000:32:2`; Snapserver transports it to clients as FLAC.
- `MA-Test-PCM` is an independent synthetic/test ingress on `14954` with the same format and transport.
- Snapcast client streaming, JSON-RPC control, and HTTP JSON-RPC use alternate ports `11704`, `11705`, and `11780`.
- Snapcast 0.35.0 Bookworm release `.deb` files are SHA-256 verified for amd64, arm64, and armhf. The Debian base image is digest-pinned; dependency resolution uses the immutable `20260810T000000Z` Debian snapshot and exact direct-dependency versions in `packages.lock`.
- The production `server` target contains no curl or Snapclient and runs as the package-created `snapserver` user. Compose drops all capabilities, forbids privilege escalation, makes the root filesystem read-only, and provides only explicit tmpfs write locations. The separate `test` target adds the pinned Snapclient.

## Run

```bash
docker compose up --build -d --wait
./scripts/rpc.py get-status
./scripts/synthetic-pcm.py                 # feed MA-Test-PCM
# With any connected Snapclient group:
./scripts/rpc.py set-stream --stream-id MA-Test-PCM
```

All host publishes bind to loopback by default. Published ports can be changed with `SNAPCAST_STREAM_PORT`, `SNAPCAST_CONTROL_PORT`, `SNAPCAST_HTTP_PORT`, `AIRPLAY_PCM_PORT`, and `MA_TEST_PCM_PORT`.

## Test

```bash
./scripts/test
```

The test profile starts a file-output Snapclient, verifies `Server.GetStatus`, successfully calls `Group.SetStream`, feeds raw 48 kHz/32-bit/stereo synthetic PCM, and proves the client decoded a frame-aligned, non-silent PCM payload of the expected minimum size. Tests use a PID-qualified Compose project and guarded cleanup, so they cannot tear down a manually running tracer. `SKIP_INTEGRATION=1 ./scripts/test` runs only package contract tests.

## Constraints

This is deliberately standalone: no authentication/TLS, discovery, AirPlay receiver, persistence volume, production ports, Kubernetes resources, or production rollout. The AirPlay bridge is expected to connect as a TCP client and emit exactly the documented raw PCM format.
