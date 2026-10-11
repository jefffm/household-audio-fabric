# Announcer

An authenticated, single-replica announcement service. Local Piper speech becomes
a finite WAV, then a one-shot MQTT5 notification tells an Immortal output to fetch
it. It does not route, pause, change the volume of, or resume music. Snapcast is
queried read-only to require connected, idle target streams before synthesis and
again before dispatch. This is not an audio-focus guarantee: music could start
after the check and stock Immortal has no reliable completion acknowledgement.

## Caller contract

Behind a private TLS gateway, `POST /v1/announcements` requires a Bearer token,
`Content-Type: application/json`, and a caller-scoped `Idempotency-Key` (1–128
letters, digits, `_ . : -`). Body, with no additional keys:

```json
{"topic":"door.open","text":"The side door opened.","ttl_seconds":30}
```

202 means durable acceptance, not playback. `GET /v1/announcements/{id}` returns
that caller's status and per-output results, never the text. Another caller gets
404. Reusing a key with different input returns409; identical retries preserve
the original deadline, including after expiration/restart. Keys/results are kept
24 hours; do not retry older events. Request text is cleared after processing or
expiry, but SQLite/WAL and backups are not a secure-erasure mechanism.

Limits: 2048-byte body, 240 characters, 1–30-second lifetime, eight waiting jobs,
eight newly accepted requests per caller per minute, 10000 retained jobs, one
global dispatch lane, 15-second synthesis deadline and10-second maximum speech.
Excess speech is rejected, not truncated. A native inference timeout kills its
child process. Busy/offline or unreadable Snapcast status fails closed until
request expiry. One unavailable subscriber holds the broadcast until expiry.

Every accepted request snapshots its configured subscribers and is reauthorized
before work starts. Caller input cannot choose devices, URLs, MQTT topics,
commands, gain or priority. Changing configuration requires a service restart.

Statuses: `accepted`, `synthesizing`, `published_unconfirmed`,
`media_fetched_unconfirmed`, `expired`, `failed`, `uncertain`. Per-output results
are authoritative for mixed broadcasts; the aggregate prioritizes uncertainty
and failures. A successful broker PUBACK does not establish device receipt;
`media_fetched_unconfirmed` means the server wrote a whole WAV response, not that
the device decoded or audibly played it. Partial range reads do not upgrade status.

The ledger commits dispatch intent before MQTT publication. A fresh MQTT5 client
publishes once, without retention or automatic reconnect/replay, with bounded
message expiry. PUBACK failure/no-subscriber codes are not successes. Ambiguous
disconnects/crashes become `uncertain` and are never retried automatically.
The lane remains reserved until request expiry plus12 seconds, even after
restart, to reduce overlapping preparation/playback. This intentionally drops
bursts that expire while waiting; it cannot bound Android's internal buffering.

## Output and network boundary

The media listener accepts only exact `/media/{id}/{output}.wav` paths from that
output's configured IPv4 source, for a dispatched request before its deadline.
It ignores forwarded-IP headers, serves no arbitrary files/URLs, has no redirects,
and allows at most16 GETs per output/clip. Byte ranges and HEAD are supported.
Files are removed after expiry. URLs contain identifiers, **not credentials**:
Immortal logs URLs and cannot attach authentication headers. An isolated private
LAN and a source/destination/port firewall rule are required. IP identity is not
cryptographic device identity. Never expose the media listener through ingress.

Piper produces PCM16 mono22.05kHz WAV, attenuated to peak ≤3932 (about−18.4dBFS).
Player gain is capped at0.1. Android's existing alarm-volume/DND settings still
apply; the service never raises them or unmutes anything. Full-volume control,
deterministic ducking, receiver deduplication/expiry and playback ACKs need a
hardened output agent, not promises layered onto stock Immortal.

## Configuration and runtime

Set `ANNOUNCER_CONFIG` to a private JSON file. Example addresses are reserved;
replace only in deployment configuration. Use a random high-entropy caller token
and store its SHA-256 here, not the raw token. Broker credentials are separate.

```json
{
  "api_bind":"0.0.0.0", "api_port":18084,
  "media_bind":"192.0.2.5", "media_port":18083,
  "data_dir":"/data", "model":"/opt/voice/en_US-ljspeech-medium.onnx",
  "media_base_url":"http://192.0.2.5:18083",
  "snapcast_url":"http://192.0.2.5:11780/jsonrpc",
  "mqtt":{"host":"127.0.0.1","port":1883,"username":"announcer","password_file":"/mqtt/password"},
  "callers":{"door":{"token_sha256":"REPLACE_WITH_64_HEX_SHA256","topics":["door.open"]}},
  "outputs":{"speaker":{"topics":["door.open"],"source_ip":"192.0.2.90","notify_topic":"outputs/example/notify/set","snapcast_client_id":"example-client","volume":0.1}}
}
```

Run one replica with a persistent local filesystem, writable `/data`, read-only
root and no capabilities. A filesystem lock rejects a second process on the same
database. `/healthz` reports worker liveness, not broker/device readiness. Host
networking may be needed to preserve source IP; Kubernetes NetworkPolicy is not
a substitute for the host firewall in that mode. The raw API must be unavailable
to LAN clients; terminate TLS at the private gateway. Do not log request bodies,
Bearer headers or media URLs in the gateway.

## Build and test

Python3.12 and Podman are required for the complete suite. No real speakers are
used; the integration tests launch a loopback-only disposable Mosquitto and
generate real speech. They also kill a service after broker receipt but before
PUBACK and verify no restart replay.

```sh
python3.12 -m venv .venv
.venv/bin/pip install --require-hashes -r requirements.lock
.venv/bin/python fetch_voice.py voice
ANNOUNCER_TEST_MODEL="$PWD/voice/en_US-ljspeech-medium.onnx" .venv/bin/python -m unittest discover -s tests -v
podman build -f Containerfile -t household-announcer:test .
```

Without `ANNOUNCER_TEST_MODEL`, speech/broker integration is explicitly skipped.
See LICENSES.md and voices.lock.json before redistributing or changing voices.
