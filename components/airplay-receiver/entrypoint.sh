#!/bin/sh
set -eu

role=${1:-shairport}
[ "$#" -eq 0 ] || shift
cap_eff() { hex=$(awk '/^CapEff:/ {print $2}' /proc/self/status); [ $((0x$hex & $1)) -ne 0 ]; }
require_rt() {
  [ "${AIRPLAY_TEST_SKIP_CAP_PREFLIGHT:-}" != 1 ] || return 0
  cap_eff 8388608 || { echo "CAP_SYS_NICE is required for SCHED_FIFO" >&2; exit 77; }
  limit=$(ulimit -r)
  [ "$limit" = unlimited ] || [ "$limit" -ge "$1" ] || { echo "rtprio >= $1 is required" >&2; exit 77; }
}

require_boundary() {
  iface=${AIRPLAY_INTERFACE:?AIRPLAY_INTERFACE is required}
  v4=${AIRPLAY_ALLOWED_IPV4_CIDR:?AIRPLAY_ALLOWED_IPV4_CIDR is required}
  v6=${AIRPLAY_ALLOWED_IPV6_CIDR:?AIRPLAY_ALLOWED_IPV6_CIDR is required}
  boundary=${AIRPLAY_BOUNDARY_ATTESTATION:-/run/airplay-boundary/verified}
  case "$iface" in *[!A-Za-z0-9_.:-]*|'') echo "AIRPLAY_INTERFACE is invalid" >&2; exit 64;; esac
  [ -d "/sys/class/net/$iface" ] || { echo "AIRPLAY_INTERFACE does not exist: $iface" >&2; exit 78; }
  grep -Eq "^interface=$iface ipv4=$v4 ipv6=$v6 ruleset_sha256=[0-9a-f]{64}$" "$boundary" 2>/dev/null || { echo "verified host firewall boundary is absent or mismatched" >&2; exit 78; }
}

prepare_shairport() {
  [ -S /run/dbus/system_bus_socket ] || { echo "host system D-Bus socket is required at /run/dbus/system_bus_socket" >&2; exit 78; }
  name=${AIRPLAY_NAME:-Household Audio}
  id=${AIRPLAY_DEVICE_ID:?AIRPLAY_DEVICE_ID must be a stable SOPS-managed 12-hex-digit value}
  require_boundary
  case "$name" in *[!A-Za-z0-9._\ \-]*) echo "AIRPLAY_NAME contains unsupported characters" >&2; exit 64;; esac
  case "$id" in *[!0-9A-Fa-f]*|'') echo "AIRPLAY_DEVICE_ID must contain exactly 12 hex digits" >&2; exit 64;; esac
  [ "${#id}" -eq 12 ] || { echo "AIRPLAY_DEVICE_ID must contain exactly 12 hex digits" >&2; exit 64; }

  backend=${AIRPLAY_OUTPUT_BACKEND:-stdout}
  case "$backend" in
    stdout) ;;
    pipe)
      audio_fifo=/run/airplay/audio
      if [ -e "$audio_fifo" ] || [ -L "$audio_fifo" ]; then
        [ -p "$audio_fifo" ] && [ ! -L "$audio_fifo" ] || { echo "audio output path is not a FIFO: $audio_fifo" >&2; exit 73; }
        chmod 0600 "$audio_fifo"
      else
        (umask 077 && mkfifo "$audio_fifo") || { echo "could not create audio output FIFO: $audio_fifo" >&2; exit 73; }
      fi
      ;;
    *) echo "AIRPLAY_OUTPUT_BACKEND must be stdout or pipe" >&2; exit 64;;
  esac

  escaped=$(printf '%s' "$name" | sed 's/[&|]/\&/g')
  config=${SHAIRPORT_CONFIG:-/run/airplay/shairport-sync.conf}
  if [ -e /run/airplay/metadata ] || [ -L /run/airplay/metadata ]; then
    [ -p /run/airplay/metadata ] && [ ! -L /run/airplay/metadata ] || { echo "metadata path is not a FIFO" >&2; exit 73; }
    chmod 0600 /run/airplay/metadata
  else
    (umask 077 && mkfifo /run/airplay/metadata) || { echo "could not create metadata FIFO" >&2; exit 73; }
  fi
  sed -e "s|@@AIRPLAY_NAME@@|$escaped|g" -e "s|@@AIRPLAY_DEVICE_ID@@|$id|g" -e "s|@@AIRPLAY_INTERFACE@@|$iface|g" -e "s|@@OUTPUT_BACKEND@@|$backend|g" /etc/shairport-sync.conf.in > "$config"
}

case "$role" in
  prepare)
    [ "$#" -eq 0 ] || { echo "prepare does not accept arguments" >&2; exit 64; }
    prepare_shairport
    ;;
  shairport)
    require_rt 3
    prepare_shairport
    exec /usr/local/bin/shairport-sync -c "${SHAIRPORT_CONFIG:-/run/airplay/shairport-sync.conf}" "$@"
    ;;
  nqptp)
    require_boundary
    require_rt 5
    [ "${AIRPLAY_TEST_SKIP_CAP_PREFLIGHT:-}" = 1 ] || cap_eff 1024 || { echo "CAP_NET_BIND_SERVICE is required for UDP 319/320" >&2; exit 77; }
    exec /usr/local/bin/nqptp "$@"
    ;;
  *) echo "usage: entrypoint.sh {prepare|shairport|nqptp} [arguments...]" >&2; exit 64;;
esac
