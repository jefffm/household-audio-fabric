#!/usr/bin/env bash
set -euo pipefail
router_dir=$(cd "$(dirname "$0")/.." && pwd)
snap_dir=$(cd "$router_dir/../snapserver" && pwd)
project="router-live-$$"
read -r SNAPCAST_STREAM_PORT SNAPCAST_CONTROL_PORT SNAPCAST_HTTP_PORT AIRPLAY_PCM_PORT MA_TEST_PCM_PORT < <(python3 - <<'PY'
import socket
ports=[]
for _ in range(5):
    s=socket.socket(); s.bind(('127.0.0.1',0)); ports.append(s.getsockname()[1]); s.close()
print(*ports)
PY
)
export SNAPCAST_STREAM_PORT SNAPCAST_CONTROL_PORT SNAPCAST_HTTP_PORT AIRPLAY_PCM_PORT MA_TEST_PCM_PORT
live_url="http://127.0.0.1:${SNAPCAST_HTTP_PORT}/jsonrpc"
cleanup() { docker compose -p "$project" -f "$snap_dir/compose.yaml" --profile test down -v --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM
docker compose -p "$project" -f "$snap_dir/compose.yaml" --profile test up --build -d
for _ in $(seq 1 60); do
  group=$(LIVE_URL="$live_url" python3 - <<'PY' 2>/dev/null || true
import json, os, urllib.request
req=urllib.request.Request(os.environ['LIVE_URL'],data=json.dumps({'jsonrpc':'2.0','id':1,'method':'Server.GetStatus','params':{}}).encode(),headers={'Content-Type':'application/json','Connection':'close'})
body=json.load(urllib.request.urlopen(req,timeout=2))
groups=body['result']['server']['groups']
print(groups[0]['id'] if groups else '')
PY
)
  [[ -n "$group" ]] && break
  sleep 1
done
[[ -n "${group:-}" ]] || { echo 'Snapcast group did not appear' >&2; exit 1; }
cd "$router_dir"
SNAPCAST_LIVE_URL="$live_url" SNAPCAST_LIVE_GROUP="$group" cargo test --locked --test live_snapcast -- --ignored --nocapture
