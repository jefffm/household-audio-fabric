#!/bin/sh
set -eu
mode=${1:-verify}; iface=${AIRPLAY_INTERFACE:?}; v4=${AIRPLAY_ALLOWED_IPV4_CIDR:?}; v6=${AIRPLAY_ALLOWED_IPV6_CIDR:?}
dir=$(dirname "$0"); out=${AIRPLAY_NFT_RENDERED:-/run/household-audio-airplay.nft}; attest=${AIRPLAY_BOUNDARY_ATTESTATION:-/run/airplay-boundary/verified}
rm -f "$attest"; mkdir -p "$(dirname "$out")" "$(dirname "$attest")"
case "$iface" in *[!A-Za-z0-9_.:-]*|'') exit 64;; esac
ip link show dev "$iface" >/dev/null 2>&1 || { echo "interface not found: $iface" >&2; exit 78; }
ip -j addr show dev "$iface" | python3 -c 'import ipaddress,json,sys; d=json.load(sys.stdin); nets=[ipaddress.ip_network(x) for x in sys.argv[1:]]; addrs=[ipaddress.ip_address(a["local"]) for x in d for a in x.get("addr_info",[])]; assert all(any(a in n for a in addrs) for n in nets)' "$v4" "$v6" || { echo "interface addresses do not cover both allowed-network CIDRs" >&2; exit 78; }
sed -e "s|@@INTERFACE@@|$iface|g" -e "s|@@IPV4_CIDR@@|$v4|g" -e "s|@@IPV6_CIDR@@|$v6|g" "$dir/household-audio-airplay.nft.in" > "$out"
expected=$(mktemp); actual=$(mktemp); batch=$(mktemp); trap 'rm -f "$expected" "$actual" "$batch"' EXIT
unshare -n sh -c "nft -f '$out'; nft -j list table inet household_audio_airplay" | "$dir/canonicalize-nft.py" > "$expected"
if [ "$mode" = apply ]; then
  if nft list table inet household_audio_airplay >/dev/null 2>&1; then
    { echo 'delete table inet household_audio_airplay'; cat "$out"; } > "$batch"
  else cp "$out" "$batch"; fi
  nft -f "$batch"
elif [ "$mode" != verify ]; then echo 'usage: install-firewall.sh {apply|verify}' >&2; exit 64; fi
nft -j list table inet household_audio_airplay | "$dir/canonicalize-nft.py" > "$actual"
cmp -s "$expected" "$actual" || { echo 'active ruleset has missing, reordered, or extra rules' >&2; exit 78; }
hash=$(sha256sum "$expected" | cut -d' ' -f1)
printf 'interface=%s ipv4=%s ipv6=%s ruleset_sha256=%s\n' "$iface" "$v4" "$v6" "$hash" > "$attest"; chmod 0444 "$attest"
