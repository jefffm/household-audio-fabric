#!/bin/sh
set -eu
target=${1:?usage: relay-entrypoint.sh TCP:host:port}
case "$target" in TCP:*:[0-9]*) ;; *) echo "relay target must be TCP:host:port" >&2; exit 64;; esac
printf '%s' "$target" > /tmp/relay-target
exec socat -u STDIN "$target"
