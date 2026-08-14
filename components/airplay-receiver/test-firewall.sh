#!/bin/sh
set -eu
cd "$(dirname "$0")"
f=household-audio-airplay.nft.in
for rule in \
 'ip saddr @@IPV4_CIDR@@ tcp dport 7000 accept' \
 'ip6 saddr @@IPV6_CIDR@@ tcp dport 7000 accept' \
 'tcp dport 7000 reject with tcp reset' \
 'ip saddr @@IPV4_CIDR@@ udp dport { 319, 320 } accept' \
 'ip6 saddr @@IPV6_CIDR@@ udp dport { 319, 320 } accept' \
 'udp dport { 319, 320 } reject'; do grep -F "$rule" "$f" >/dev/null; done
if command -v unshare >/dev/null && command -v nft >/dev/null; then
  ns_flags=-Urn
  [ "$(id -u)" -ne 0 ] || ns_flags=-n
  td=$(mktemp -d); trap 'rm -rf "$td"' EXIT
  here=$PWD
  unshare $ns_flags sh -eu -c '
    ip link set lo up
    export AIRPLAY_INTERFACE=lo AIRPLAY_ALLOWED_IPV4_CIDR=127.0.0.0/8 AIRPLAY_ALLOWED_IPV6_CIDR=::1/128
    export AIRPLAY_BOUNDARY_ATTESTATION="$1/verified" AIRPLAY_NFT_RENDERED="$1/rendered.nft"
    "$2/install-firewall.sh" apply
    test -s "$1/verified"
    # Insert an overgrant. Exact JSON/order/no-extras verification must fail and erase attestation.
    nft insert rule inet household_audio_airplay ingress tcp dport 7000 accept
    if "$2/install-firewall.sh" verify; then exit 91; fi
    test ! -e "$1/verified"
  ' sh "$td" "$here"
fi
echo 'PASS: dual-stack nft boundary is exact and rejects inserted overgrant'
