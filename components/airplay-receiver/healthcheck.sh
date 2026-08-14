#!/bin/sh
set -eu
cmd=$(tr '\000' ' ' < /proc/1/cmdline)
has_port() { grep -Eq ":[0]*$1[[:space:]]" "$2"; }
boundary_ok() {
  iface=${AIRPLAY_INTERFACE:?}; v4=${AIRPLAY_ALLOWED_IPV4_CIDR:?}; v6=${AIRPLAY_ALLOWED_IPV6_CIDR:?}
  test -d "/sys/class/net/$iface"
  grep -Eq "^interface=$iface ipv4=$v4 ipv6=$v6 ruleset_sha256=[0-9a-f]{64}$" "${AIRPLAY_BOUNDARY_ATTESTATION:-/run/airplay-boundary/verified}"
}
has_fifo_policy() { grep -qsE '^policy[[:space:]]*:[[:space:]]*1$' /proc/1/task/*/sched; }
case "$cmd" in
  *shairport-sync*)
    boundary_ok
    test -r /run/airplay/shairport-sync.conf
    test -p /run/airplay/metadata
    test -S /run/dbus/system_bus_socket
    test -e /dev/shm/nqptp
    has_port 1B58 /proc/net/tcp || has_port 1B58 /proc/net/tcp6
    kill -0 1
    ;;
  *nqptp*)
    boundary_ok
    has_port 013F /proc/net/udp || has_port 013F /proc/net/udp6
    has_port 0140 /proc/net/udp || has_port 0140 /proc/net/udp6
    test -e /dev/shm/nqptp
    has_fifo_policy
    kill -0 1
    ;;
  *) exit 1 ;;
esac
