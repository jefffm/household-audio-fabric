#!/bin/sh
set -eu
cd "$(dirname "$0")"
fail() { echo "FAIL: $*" >&2; exit 1; }
assert_grep() { [ "$1" != -- ] || shift; grep -Eq -- "$1" "$2" || fail "$1 missing from $2"; }

assert_grep 'ARG SHAIRPORT_SYNC_VERSION=5\.2\.1' Containerfile
assert_grep 'ARG NQPTP_VERSION=1\.2\.8' Containerfile
assert_grep -- '--with-airplay-2' Containerfile
assert_grep -- '--with-metadata' Containerfile
assert_grep -- '--with-metadata-pipe' Containerfile
assert_grep -- '--with-avahi' Containerfile
assert_grep -- '--with-ssl=openssl' Containerfile
assert_grep -- '--with-stdout' Containerfile
assert_grep -- '--with-pipe' Containerfile
assert_grep -- '--with-ffmpeg' Containerfile
for expected in 'service_type = "airplay2"' 'output_backend = "@@OUTPUT_BACKEND@@"' 'output_rate = 48000' 'output_format = "S32_LE"' 'output_channels = 2' 'ignore_volume_control = "yes"' 'include_cover_art = "no"' 'pipe_name = "/run/airplay/metadata"' 'interface = "@@AIRPLAY_INTERFACE@@"'; do assert_grep "$expected" shairport-sync.conf.in; done
assert_grep '^pipe = \{' shairport-sync.conf.in
assert_grep 'name = "/run/airplay/audio"' shairport-sync.conf.in
assert_grep 'AIRPLAY_OUTPUT_BACKEND must be stdout or pipe' entrypoint.sh
assert_grep 'RELAY_INPUT_FIFO must name an existing, non-symlink FIFO' relay-entrypoint.sh
assert_grep 'RELAY_INPUT_FIFO must be mode 0600 and owned by the relay UID' relay-entrypoint.sh
! grep -q log_output_to shairport-sync.conf.in || fail 'obsolete log directive retained'
assert_grep '^USER 10001:10001$' Containerfile
assert_grep 'CAP_SYS_NICE' entrypoint.sh
assert_grep 'CAP_NET_BIND_SERVICE' entrypoint.sh
assert_grep 'prepare does not accept arguments' entrypoint.sh
assert_grep 'rtprio >= \$1 is required' entrypoint.sh
assert_grep 'AIRPLAY_DEVICE_ID.*12' entrypoint.sh
assert_grep 'AIRPLAY_INTERFACE is required' entrypoint.sh
assert_grep 'verified host firewall boundary' entrypoint.sh
assert_grep 'ip -j addr show dev' install-firewall.sh
assert_grep 'INTEGRATION TEST ONLY' compose.integration-test-only.yaml
assert_grep 'AIRPLAY_TEST_CAP_IMAGE' compose.integration-test-only.yaml
assert_grep 'network_mode: host' compose.integration-test-only.yaml
assert_grep 'no-new-privileges:true' compose.integration-test-only.yaml
assert_grep 'cap_drop: \[ALL\]' compose.integration-test-only.yaml
assert_grep 'read_only: true' compose.integration-test-only.yaml
assert_grep 'iifname "@@INTERFACE@@" ip saddr @@IPV4_CIDR@@ tcp dport 7000 accept' household-audio-airplay.nft.in
assert_grep 'ip6 saddr @@IPV6_CIDR@@ tcp dport 7000 accept' household-audio-airplay.nft.in
assert_grep 'tcp dport 7000 reject' household-audio-airplay.nft.in
assert_grep 'udp dport \{ 319, 320 \} reject' household-audio-airplay.nft.in
assert_grep 'NQPTP is GPL-2.0-only' LICENSES.md
assert_grep 'org.opencontainers.image.source="https://github.com/jefffm/household-audio-fabric"' Containerfile
assert_grep 'DocumentNamespace:.*\$\{runtime_arch\}' generate-package-inventory.sh
assert_grep 'DEPENDS_ON \$libavcodec_id' generate-package-inventory.sh
assert_grep 'DEPENDS_ON \$libavahi_id' generate-package-inventory.sh
assert_grep 'DEPENDS_ON \$libc_id' generate-package-inventory.sh
! grep -q 'amd64-amd64' generate-package-inventory.sh || fail 'package inventory hard-codes amd64 dependency IDs'
assert_grep 'deliberately is \*\*not\*\* described as containing only those two executables' README.md
for x in entrypoint.sh healthcheck.sh relay-entrypoint.sh relay-healthcheck.sh generate-package-inventory.sh integration-test.sh install-firewall.sh test-firewall.sh test-firewall-packets.sh test-entrypoints.sh relay-integration-test.sh; do [ -x "$x" ] || fail "$x not executable"; sh -n "$x"; done
./test-firewall.sh
./test-firewall-packets.sh
./test-entrypoints.sh
if grep -Eq 'setcap .*usr/local/bin/(nqptp|shairport)' Containerfile; then fail 'production image embeds file capabilities'; fi

if [ -n "${IMAGE:-}" ]; then
  uid=$(docker run --rm --entrypoint id "$IMAGE" -u); [ "$uid" = 10001 ] || fail "image UID is $uid"
  version=$(docker run --rm --entrypoint sh "$IMAGE" -c 'shairport-sync -V')
  printf '%s' "$version" | grep -q '5.2.1-AirPlay2' || fail "wrong version: $version"
  printf '%s' "$version" | grep -Eq 'OpenSSL-Avahi-stdout-pipe-soxr-metadata' || fail "missing features: $version"
  nqversion=$(docker run --rm --entrypoint sh "$IMAGE" -c 'nqptp -V 2>&1 || true'); printf '%s' "$nqversion" | grep -q '1.2.8' || fail 'wrong NQPTP version'
  docker run --rm --entrypoint sh "$IMAGE" -c 'ldd /usr/local/bin/shairport-sync | grep -q libavcodec'
  for forbidden in ffmpeg ffprobe socat avahi-daemon dbus-daemon setcap; do docker run --rm --entrypoint sh "$IMAGE" -c "! command -v $forbidden" || fail "$forbidden leaked into receiver"; done
  docker run --rm --entrypoint sh "$IMAGE" -c 'test -f /usr/share/doc/shairport-sync/COPYING && test -f /usr/share/doc/nqptp/LICENSE && test -f /usr/share/doc/household-audio-airplay/package-inventory.spdx'
  packages=$(docker run --rm --entrypoint sh "$IMAGE" -c "grep -c '^PackageName:' /usr/share/doc/household-audio-airplay/package-inventory.spdx")
  [ "$packages" -gt 20 ] || fail 'package inventory is incomplete'
  docker run --rm --entrypoint sh "$IMAGE" -c "grep -q 'Relationship: SPDXRef-DOCUMENT DESCRIBES SPDXRef-Package-receiver-image' /usr/share/doc/household-audio-airplay/package-inventory.spdx && grep -q 'Relationship: SPDXRef-Package-receiver-image DESCENDANT_OF SPDXRef-Package-base' /usr/share/doc/household-audio-airplay/package-inventory.spdx && grep -q 'Relationship: SPDXRef-Package-receiver-image CONTAINS' /usr/share/doc/household-audio-airplay/package-inventory.spdx && grep -q 'PackageLicenseDeclared: GPL-2.0-only' /usr/share/doc/household-audio-airplay/package-inventory.spdx"
  docker run --rm --entrypoint sh "$IMAGE" -c "grep -q 'PackageName: libavcodec59' /usr/share/doc/household-audio-airplay/package-inventory.spdx && grep -q 'PackageName: shairport-sync' /usr/share/doc/household-audio-airplay/package-inventory.spdx"
  image_arch=$(docker run --rm --entrypoint dpkg "$IMAGE" --print-architecture)
  docker run --rm --entrypoint sh "$IMAGE" -c "grep -qE 'DocumentNamespace: .*[/-]$image_arch$' /usr/share/doc/household-audio-airplay/package-inventory.spdx" || fail "inventory namespace does not match $image_arch"
  # Missing runtime grants must fail closed, not silently lose realtime/timing correctness.
  set +e
  docker run --rm -e AIRPLAY_INTERFACE=lo -e AIRPLAY_ALLOWED_IPV4_CIDR=127.0.0.0/8 -e AIRPLAY_ALLOWED_IPV6_CIDR=::1/128 "$IMAGE" nqptp >/dev/null 2>&1; nqrc=$?
  docker run --rm -e AIRPLAY_DEVICE_ID=bad "$IMAGE" shairport >/dev/null 2>&1; rxrc=$?
  set -e
  [ "$nqrc" -eq 78 ] || fail "NQPTP missing-boundary exit was $nqrc"
  [ "$rxrc" -eq 77 ] || fail "Shairport missing-realtime exit was $rxrc"
fi

echo 'PASS: household-audio-airplay static/image tests'
