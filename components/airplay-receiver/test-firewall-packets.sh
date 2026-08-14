#!/bin/sh
set -eu
command -v unshare >/dev/null && command -v socat >/dev/null || { echo 'SKIP: packet tools unavailable'; exit 0; }
here=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
ns_flags=-Urn
[ "$(id -u)" -ne 0 ] || ns_flags=-n
unshare $ns_flags sh -eu -c '
  p1= p2=; cleanup(){ [ -z "$p1" ] || kill "$p1" 2>/dev/null ||:; [ -z "$p2" ] || kill "$p2" 2>/dev/null ||:; }; trap cleanup EXIT
  ip link set lo up
  unshare -n sleep 60 & p1=$!; ip link add audio0 type veth peer name peer0; ip addr add 192.0.2.1/24 dev audio0; ip -6 addr add 2001:db8:1::1/64 dev audio0 nodad; ip link set audio0 up; ip link set peer0 netns $p1
  nsenter -t $p1 -n ip link set lo up; nsenter -t $p1 -n ip link set peer0 up; nsenter -t $p1 -n ip addr add 192.0.2.2/24 dev peer0; nsenter -t $p1 -n ip -6 addr add 2001:db8:1::2/64 dev peer0 nodad; nsenter -t $p1 -n ip addr add 198.51.100.2/32 dev peer0; nsenter -t $p1 -n ip -6 addr add 2001:db8:2::2/128 dev peer0 nodad
  unshare -n sleep 60 & p2=$!; ip link add tailscale0 type veth peer name tailpeer; ip addr add 192.0.2.3/24 dev tailscale0; ip -6 addr add 2001:db8:1::3/64 dev tailscale0 nodad; ip link set tailscale0 up; ip link set tailpeer netns $p2
  nsenter -t $p2 -n ip link set lo up; nsenter -t $p2 -n ip link set tailpeer up; nsenter -t $p2 -n ip addr add 192.0.2.4/24 dev tailpeer; nsenter -t $p2 -n ip -6 addr add 2001:db8:1::4/64 dev tailpeer nodad
  sed -e s/@@INTERFACE@@/audio0/g -e s,@@IPV4_CIDR@@,192.0.2.0/24,g -e s,@@IPV6_CIDR@@,2001:db8:1::/64,g "$1/household-audio-airplay.nft.in" | nft -f -
  # Correct-interface/CIDR TCP, IPv4 and IPv6.
  timeout 3 socat -u TCP4-LISTEN:7000,reuseaddr /dev/null & l=$!; sleep .1; echo ok | timeout 3 nsenter -t $p1 -n socat -u - TCP4:192.0.2.1:7000; wait $l
  timeout 3 socat -u TCP6-LISTEN:7000,reuseaddr /dev/null & l=$!; sleep .1; echo ok | timeout 3 nsenter -t $p1 -n socat -u - TCP6:[2001:db8:1::1]:7000; wait $l
  # Wrong source CIDR on the selected interface is rejected for both families.
  timeout 3 socat -u TCP4-LISTEN:7000,reuseaddr /dev/null & l=$!; sleep .1; if echo bad | timeout 3 nsenter -t $p1 -n socat -u -T1 - TCP4:192.0.2.1:7000,bind=198.51.100.2; then exit 79; fi; kill $l 2>/dev/null ||:
  timeout 3 socat -u TCP6-LISTEN:7000,reuseaddr /dev/null & l=$!; sleep .1; if echo bad | timeout 3 nsenter -t $p1 -n socat -u -T1 - TCP6:[2001:db8:1::1]:7000,bind=[2001:db8:2::2]; then exit 80; fi; kill $l 2>/dev/null ||:
  # TCP on Tailscale/other interface is rejected for both families.
  timeout 3 socat -u TCP4-LISTEN:7000,reuseaddr /dev/null & l=$!; sleep .1; if echo bad | timeout 3 nsenter -t $p2 -n socat -u -T1 - TCP4:192.0.2.3:7000; then exit 81; fi; kill $l 2>/dev/null ||:
  timeout 3 socat -u TCP6-LISTEN:7000,reuseaddr /dev/null & l=$!; sleep .1; if echo bad | timeout 3 nsenter -t $p2 -n socat -u -T1 - TCP6:[2001:db8:1::3]:7000; then exit 82; fi; kill $l 2>/dev/null ||:
  # Both UDP ports, both families: correct ingress reaches listener; other ingress does not.
  for port in 319 320; do
    rm -f /tmp/got; timeout 2 socat -u UDP4-RECVFROM:$port,reuseaddr CREATE:/tmp/got & l=$!; sleep .1; echo ok | nsenter -t $p1 -n socat -u - UDP4:192.0.2.1:$port; wait $l; grep -q ok /tmp/got
    rm -f /tmp/got; timeout 1 socat -u UDP4-RECVFROM:$port,reuseaddr CREATE:/tmp/got & l=$!; sleep .1; echo bad | nsenter -t $p2 -n socat -u - UDP4:192.0.2.3:$port ||:; wait $l && exit 83 ||:; test ! -s /tmp/got
    rm -f /tmp/got; timeout 1 socat -u UDP4-RECVFROM:$port,reuseaddr CREATE:/tmp/got & l=$!; sleep .1; echo bad | nsenter -t $p1 -n socat -u - UDP4:192.0.2.1:$port,bind=198.51.100.2 ||:; wait $l && exit 85 ||:; test ! -s /tmp/got
    rm -f /tmp/got; timeout 2 socat -u UDP6-RECVFROM:$port,reuseaddr CREATE:/tmp/got & l=$!; sleep .1; echo ok | nsenter -t $p1 -n socat -u - UDP6:[2001:db8:1::1]:$port; wait $l; grep -q ok /tmp/got
    rm -f /tmp/got; timeout 1 socat -u UDP6-RECVFROM:$port,reuseaddr CREATE:/tmp/got & l=$!; sleep .1; echo bad | nsenter -t $p2 -n socat -u - UDP6:[2001:db8:1::3]:$port ||:; wait $l && exit 84 ||:; test ! -s /tmp/got
    rm -f /tmp/got; timeout 1 socat -u UDP6-RECVFROM:$port,reuseaddr CREATE:/tmp/got & l=$!; sleep .1; echo bad | nsenter -t $p1 -n socat -u - UDP6:[2001:db8:1::1]:$port,bind=[2001:db8:2::2] ||:; wait $l && exit 86 ||:; test ! -s /tmp/got
  done
' sh "$here"
echo 'PASS: dual-stack TCP/UDP packet boundary accepts allowed-network ingress and rejects wrong-CIDR and Tailscale/other ingress'
