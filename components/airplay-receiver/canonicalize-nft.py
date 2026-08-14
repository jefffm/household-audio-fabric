#!/usr/bin/env python3
import json, sys
obj=json.load(sys.stdin)
out=[]
for item in obj.get("nftables", []):
    if "metainfo" in item: continue
    item=json.loads(json.dumps(item))
    for value in item.values():
        if isinstance(value, dict): value.pop("handle", None)
    out.append(item)
json.dump({"nftables":out},sys.stdout,sort_keys=True,separators=(",",":"))
sys.stdout.write("\n")
