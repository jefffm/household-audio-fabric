#!/bin/sh
set -eu
cd "$(dirname "$0")"
fail() { echo "FAIL: $*" >&2; exit 1; }
tmp=$(mktemp -d)
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT INT TERM
mkdir "$tmp/bin"
cat > "$tmp/bin/socat" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$MOCK_SOCAT_ARGS"
EOF
chmod +x "$tmp/bin/socat"
export PATH="$tmp/bin:$PATH" MOCK_SOCAT_ARGS="$tmp/args"

RELAY_TARGET=TCP:localhost:1704 ./relay-entrypoint.sh
[ "$(cat "$tmp/args")" = "-u
STDIN
TCP:localhost:1704" ] || fail 'relay did not preserve stdin mode'
mkfifo "$tmp/audio"; chmod 0600 "$tmp/audio"
RELAY_INPUT_FIFO="$tmp/audio" ./relay-entrypoint.sh TCP:127.0.0.1:1704
[ "$(cat "$tmp/args")" = "-u
OPEN:$tmp/audio
TCP:127.0.0.1:1704" ] || fail 'relay did not select existing FIFO'

must_fail() {
  expected=$1; shift
  set +e; "$@" >"$tmp/stdout" 2>"$tmp/stderr"; rc=$?; set -e
  [ "$rc" -eq "$expected" ] || fail "expected exit $expected, got $rc: $*"
  [ ! -s "$tmp/stdout" ] || fail "failure contaminated stdout: $*"
}
for target in '' UDP:host:1 TCP::1 TCP:bad/host:1 TCP:host:0 TCP:host:65536 TCP:host:12x TCP:host; do
  must_fail 64 ./relay-entrypoint.sh "$target"
done
: > "$tmp/regular"
must_fail 64 env RELAY_TARGET=TCP:localhost:1 ./relay-entrypoint.sh unexpected
must_fail 64 env RELAY_INPUT_FIFO=relative ./relay-entrypoint.sh TCP:localhost:1
must_fail 64 env RELAY_INPUT_FIFO=/tmp/../etc/audio ./relay-entrypoint.sh TCP:localhost:1
must_fail 66 env RELAY_INPUT_FIFO="$tmp/regular" ./relay-entrypoint.sh TCP:localhost:1
ln -s "$tmp/audio" "$tmp/link"
must_fail 66 env RELAY_INPUT_FIFO="$tmp/link" ./relay-entrypoint.sh TCP:localhost:1
chmod 0644 "$tmp/audio"
must_fail 66 env RELAY_INPUT_FIFO="$tmp/audio" ./relay-entrypoint.sh TCP:localhost:1

echo 'PASS: relay entrypoint validation and stdin/FIFO selection'
