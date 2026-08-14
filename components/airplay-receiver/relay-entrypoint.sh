#!/bin/sh
set -eu

usage() { echo "usage: relay-entrypoint.sh TCP:host:port" >&2; exit 64; }
if [ -n "${RELAY_TARGET:-}" ]; then
  [ "$#" -eq 0 ] || usage
  target=$RELAY_TARGET
else
  [ "$#" -eq 1 ] || usage
  target=$1
fi

case "$target" in
  TCP:*) endpoint=${target#TCP:} ;;
  *) echo "relay target must be TCP:host:port" >&2; exit 64;;
esac
host=${endpoint%:*}
port=${endpoint##*:}
[ "$host" != "$endpoint" ] && [ -n "$host" ] || { echo "relay target must be TCP:host:port" >&2; exit 64; }
case "$host" in *[!A-Za-z0-9._-]*|'') echo "relay TCP host is invalid" >&2; exit 64;; esac
case "$port" in *[!0-9]*|'') echo "relay TCP port is invalid" >&2; exit 64;; esac
[ "$port" -ge 1 ] 2>/dev/null && [ "$port" -le 65535 ] 2>/dev/null || { echo "relay TCP port is invalid" >&2; exit 64; }

fifo=${RELAY_INPUT_FIFO:-}
if [ -n "$fifo" ]; then
  case "$fifo" in
    /*[!A-Za-z0-9_./-]*|*/../*|*/..|*/./*|*/.) echo "RELAY_INPUT_FIFO path is invalid" >&2; exit 64;;
    /*) ;;
    *) echo "RELAY_INPUT_FIFO must be an absolute path" >&2; exit 64;;
  esac
  [ -p "$fifo" ] && [ ! -L "$fifo" ] || { echo "RELAY_INPUT_FIFO must name an existing, non-symlink FIFO" >&2; exit 66; }
  [ "$(stat -c %a "$fifo")" = 600 ] && [ "$(stat -c %u "$fifo")" = "$(id -u)" ] || { echo "RELAY_INPUT_FIFO must be mode 0600 and owned by the relay UID" >&2; exit 66; }
  exec socat -u "OPEN:$fifo" "$target"
fi
exec socat -u STDIN "$target"
