#!/usr/bin/env python3
"""Small stdlib-only Snapcast JSON-RPC client."""
import argparse
import json
import urllib.request


def call(url, method, params=None, request_id=1):
    body = json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params or {}}).encode()
    request = urllib.request.Request(url, body, {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=5) as response:
        payload = json.load(response)
    if "error" in payload:
        raise RuntimeError(json.dumps(payload["error"], sort_keys=True))
    return payload["result"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("get-status", "set-stream"))
    parser.add_argument("--url", default="http://127.0.0.1:11780/jsonrpc")
    parser.add_argument("--group-id")
    parser.add_argument("--stream-id", default="MA-Test-PCM")
    args = parser.parse_args()
    status = call(args.url, "Server.GetStatus")
    if args.command == "get-status":
        print(json.dumps(status, indent=2, sort_keys=True))
        return
    groups = status["server"]["groups"]
    group_id = args.group_id or (groups[0]["id"] if groups else None)
    if not group_id:
        parser.error("no group exists; start snapclient-test or pass --group-id")
    result = call(args.url, "Group.SetStream", {"id": group_id, "stream_id": args.stream_id})
    print(json.dumps(result, indent=2, sort_keys=True))

if __name__ == "__main__":
    main()
