#!/bin/sh
set -eu
cd "$(dirname "$0")"
RELAY_IMAGE=${RELAY_IMAGE:-household-audio-relay:test}
name=airplay-relay-test-$$
tmp=$(mktemp -d)
server_pid=
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  [ -z "$server_pid" ] || kill "$server_pid" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

[ "${RELAY_SKIP_BUILD:-}" = 1 ] || docker build --target relay -t "$RELAY_IMAGE" -f Containerfile .
chmod 0777 "$tmp"
docker run --rm --user 10001:10001 -v "$tmp:/test" --entrypoint sh "$RELAY_IMAGE" -c 'umask 077; mkfifo /test/audio'
[ "$(stat -c %a "$tmp/audio")" = 600 ] || { echo 'FIFO setup permission mismatch' >&2; exit 1; }
python3 - "$tmp/port" "$tmp/received" <<'PY' &
import socket, sys
port_file, output_file = sys.argv[1:]
with socket.socket() as server:
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", 0))
    server.listen(1)
    with open(port_file, "w") as f:
        f.write(str(server.getsockname()[1]))
    connection, _ = server.accept()
    with connection, open(output_file, "wb") as output:
        while True:
            data = connection.recv(65536)
            if not data:
                break
            output.write(data)
PY
server_pid=$!
i=0; while [ ! -s "$tmp/port" ]; do i=$((i+1)); [ "$i" -lt 100 ] || { echo 'TCP sink did not start' >&2; exit 1; }; sleep .05; done
port=$(cat "$tmp/port")
python3 - "$tmp/payload" <<'PY'
import sys
with open(sys.argv[1], "wb") as f:
    f.write(bytes(range(256)) * 257 + b"\x00raw-pcm\xff\n")
PY
chmod 0644 "$tmp/payload"
docker run -d --name "$name" --network host -v "$tmp:/run/airplay:ro" -e RELAY_INPUT_FIFO=/run/airplay/audio "$RELAY_IMAGE" "TCP:127.0.0.1:$port" >"$tmp/container-id"
docker run --rm --user 10001:10001 -v "$tmp:/test" --entrypoint sh "$RELAY_IMAGE" -c 'cat /test/payload > /test/audio'
timeout 15 docker wait "$name" >/dev/null
wait "$server_pid"; server_pid=
[ "$(docker inspect -f '{{.State.ExitCode}}' "$name")" = 0 ] || { docker logs "$name" >&2; exit 1; }
docker logs "$name" >"$tmp/logs" 2>&1
[ ! -s "$tmp/logs" ] || { echo 'relay contaminated output with logs' >&2; cat "$tmp/logs" >&2; exit 1; }
cmp "$tmp/payload" "$tmp/received"
echo 'PASS: live FIFO-to-TCP relay is byte-exact and log-free'
