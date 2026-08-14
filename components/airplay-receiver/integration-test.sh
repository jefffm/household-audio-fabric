#!/bin/sh
set -eu
cd "$(dirname "$0")"
IMAGE=${IMAGE:-household-audio-airplay:test}; SUPPORT=${SUPPORT:-household-audio-airplay:integration-support}; CAPIMAGE=${CAPIMAGE:-household-audio-airplay:test-caps}
name="AirPlay Integration $$"; id=020000000001; iface=${AIRPLAY_INTERFACE:-lo}; v4=${AIRPLAY_ALLOWED_IPV4_CIDR:-127.0.0.0/8}; v6=${AIRPLAY_ALLOWED_IPV6_CIDR:-::1/128}
nq=airplay-nqptp-$$; rx=airplay-rx-$$; av=airplay-avahi-$$; busvol=airplay-bus-$$
out=$(mktemp); err=$(mktemp); df=$(mktemp); boundary=$(mktemp -d)
printf 'interface=%s ipv4=%s ipv6=%s ruleset_sha256=%064d\n' "$iface" "$v4" "$v6" 0 > "$boundary/verified"; chmod 0755 "$boundary"; chmod 0444 "$boundary/verified"
cleanup() { docker rm -f "$rx" "$nq" "$av" >/dev/null 2>&1 || true; docker volume rm "$busvol" >/dev/null 2>&1 || true; rm -rf "$out" "$err" "$df" "$boundary"; }
trap cleanup EXIT INT TERM

docker build --target receiver -t "$IMAGE" -f Containerfile .
docker build --target integration-support -t "$SUPPORT" -f Containerfile .
cat >"$df" <<EOF
FROM $SUPPORT
USER 0:0
RUN setcap cap_net_bind_service,cap_sys_nice=ep /usr/local/bin/nqptp && setcap cap_sys_nice=ep /usr/local/bin/shairport-sync
USER 10001:10001
EOF
docker build -t "$CAPIMAGE" -f "$df" .

busmount=/run/dbus:/run/dbus:ro; browse_cmd='avahi-browse -rt _airplay._tcp'
if [ "${FORCE_TEMP_AVAHI:-}" = 1 ] || [ ! -S /run/dbus/system_bus_socket ] || ! command -v avahi-browse >/dev/null 2>&1; then
  docker volume create "$busvol" >/dev/null
  docker run -d --name "$av" --network host -v "$busvol:/run/dbus" --user 0:0 --entrypoint sh "$SUPPORT" -c 'mkdir -p /run/dbus; dbus-daemon --system --fork; exec avahi-daemon --no-chroot --debug' >/dev/null
  busmount="$busvol:/run/dbus:ro"; browse_cmd="docker exec $av avahi-browse -rt _airplay._tcp"; sleep 2
fi
common_env="-e AIRPLAY_TEST_SKIP_CAP_PREFLIGHT=1 -e AIRPLAY_INTERFACE=$iface -e AIRPLAY_ALLOWED_IPV4_CIDR=$v4 -e AIRPLAY_ALLOWED_IPV6_CIDR=$v6"
common_mount="-v $boundary:/run/airplay-boundary:ro"
set +e
docker run --rm --cap-add SYS_NICE --ulimit rtprio=5 -v "$busmount" $common_mount $common_env -e AIRPLAY_NAME=Bad -e AIRPLAY_DEVICE_ID=bad --entrypoint entrypoint.sh "$CAPIMAGE" shairport >/dev/null 2>&1
bad_id_rc=$?
set -e
[ "$bad_id_rc" -eq 64 ] || { echo "invalid identity was not rejected (exit $bad_id_rc)" >&2; exit 1; }

docker run -d --name "$nq" --cap-add NET_BIND_SERVICE --cap-add SYS_NICE --network host --ipc shareable --ulimit rtprio=5 $common_mount $common_env --entrypoint entrypoint.sh "$CAPIMAGE" nqptp >/dev/null
sleep 1; docker exec "$nq" healthcheck.sh
start_receiver() {
  : >"$out"; : >"$err"
  docker run --name "$rx" --cap-add SYS_NICE --network host --ipc "container:$nq" --ulimit rtprio=5 -v "$busmount" $common_mount $common_env -e AIRPLAY_NAME="$name" -e AIRPLAY_DEVICE_ID="$id" --entrypoint entrypoint.sh "$CAPIMAGE" shairport >"$out" 2>"$err" &
  sleep 4; docker exec "$rx" healthcheck.sh; [ ! -s "$out" ]
}
record() {
  sh -c "$browse_cmd" | awk -v n="$name" -v i="$iface" '
    $1=="=" { active=($2==i && index($0,n)); if (index($0,n) && $2!=i) bad=1 }
    active { print }
    END { if (bad) exit 3 }
  '
}
start_receiver
grep -q " $iface$" /proc/net/if_inet6 || { echo "selected interface is not dual stack" >&2; exit 1; }
first=$(record); [ "$(printf '%s\n' "$first" | grep -c "^= *$iface ")" -eq 1 ]
printf '%s\n' "$first" | sed -n 's/.*address = \[\([^]]*\)\].*/\1/p' | python3 -c 'import ipaddress,socket,sys; nets=[ipaddress.ip_network(x) for x in sys.argv[1:]]; a=list(map(ipaddress.ip_address,filter(None,map(str.strip,sys.stdin)))); assert a and all(any(x in n for n in nets) for x in a); [socket.create_connection((str(x),7000),2).close() for x in a]' "$v4" "$v6"
for value in 'port = [7000]' 'protovers=1.1' 'deviceid=02:00:00:00:00:01' 'flags=0x4' 'acl=0' 'pk='; do printf '%s' "$first" | grep -F "$value" >/dev/null; done
pk1=$(printf '%s' "$first" | sed -n 's/.*"pk=\([0-9a-f]*\)".*/\1/p')
[ -n "$pk1" ]; docker rm -f "$rx" >/dev/null; sleep 2
start_receiver
second=$(record); pk2=$(printf '%s' "$second" | sed -n 's/.*"pk=\([0-9a-f]*\)".*/\1/p')
[ "$pk1" = "$pk2" ] || { echo 'advertised public key changed across restart' >&2; exit 1; }
docker exec "$rx" test -p /run/airplay/metadata

echo 'PASS: selected-interface unique modern record and identity/key stable across restart; NQPTP/ports/shm/readiness clean'
echo 'NOTE: stream-thread FIFO, pvol and PCM playback remain physical-iPhone gates.'
