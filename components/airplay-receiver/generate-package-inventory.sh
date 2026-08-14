#!/bin/sh
set -eu
out=${1:-/usr/share/doc/household-audio-airplay/package-inventory.spdx}
tmp=$(mktemp); rel=$(mktemp); trap 'rm -f "$tmp" "$rel"' EXIT
runtime_arch=$(dpkg --print-architecture)
deb_spdx_id() {
  package=$1
  name=$(dpkg-query -W -f='${binary:Package}' "$package")
  arch=$(dpkg-query -W -f='${Architecture}' "$package")
  printf 'SPDXRef-Deb-%s' "$(printf '%s' "$name-$arch" | tr -c 'A-Za-z0-9.-' '-')"
}
libavcodec_id=$(deb_spdx_id libavcodec59)
libavahi_id=$(deb_spdx_id libavahi-client3)
libc_id=$(deb_spdx_id libc6)
{
 echo 'SPDXVersion: SPDX-2.3'; echo 'DataLicense: CC0-1.0'; echo 'SPDXID: SPDXRef-DOCUMENT'
 echo 'DocumentName: household-audio-airplay-package-inventory'
 echo "DocumentNamespace: https://github.com/jefffm/household-audio-fabric/spdx/airplay-receiver/5.2.1-1.2.8-${runtime_arch}"
 echo 'Creator: Tool: generate-package-inventory.sh-2'; echo 'Created: 2026-08-03T00:00:00Z'; echo
 echo 'PackageName: household-audio-airplay-receiver-image'; echo 'SPDXID: SPDXRef-Package-receiver-image'
 echo 'PackageVersion: shairport-sync-5.2.1_nqptp-1.2.8'; echo 'PackageDownloadLocation: NOASSERTION'; echo 'FilesAnalyzed: false'
 echo 'PackageLicenseConcluded: NOASSERTION'; echo 'PackageLicenseDeclared: NOASSERTION'; echo
 echo 'Relationship: SPDXRef-DOCUMENT DESCRIBES SPDXRef-Package-receiver-image' >>"$rel"
 echo 'Relationship: SPDXRef-Package-receiver-image DESCENDANT_OF SPDXRef-Package-base' >>"$rel"
 echo 'PackageName: debian-bookworm-slim-base'; echo 'SPDXID: SPDXRef-Package-base'
 echo 'PackageVersion: sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241'
 echo 'PackageDownloadLocation: https://hub.docker.com/_/debian'; echo 'FilesAnalyzed: false'
 echo 'PackageLicenseConcluded: NOASSERTION'; echo 'PackageLicenseDeclared: NOASSERTION'; echo

 dpkg-query -W -f='${binary:Package}\t${Version}\t${Architecture}\n' | LC_ALL=C sort | while IFS="$(printf '\t')" read -r name version arch; do
   id=$(printf '%s' "$name-$arch" | tr -c 'A-Za-z0-9.-' '-')
   echo "PackageName: $name"; echo "SPDXID: SPDXRef-Deb-$id"; echo "PackageVersion: $version"
   echo 'PackageSupplier: Organization: Debian'; echo 'PackageDownloadLocation: NOASSERTION'; echo 'FilesAnalyzed: false'
   echo 'PackageLicenseConcluded: NOASSERTION'; echo 'PackageLicenseDeclared: NOASSERTION'; echo
   echo "Relationship: SPDXRef-Package-receiver-image CONTAINS SPDXRef-Deb-$id" >>"$rel"
 done
 for spec in \
  'shairport-sync 5.2.1 MIT https://github.com/mikebrady/shairport-sync/archive/refs/tags/5.2.1.tar.gz 8f97d1a6e045bc3765b10d0cd64abe467eba343af89fa1e158f7fa28b73c4ab6 /usr/local/bin/shairport-sync' \
  'nqptp 1.2.8 GPL-2.0-only https://github.com/mikebrady/nqptp/archive/refs/tags/1.2.8.tar.gz 3a2882a299c21605f53bb215ce537f9cc7a1e894476f639ab28562c68fd183a9 /usr/local/bin/nqptp'; do
   set -- $spec; binsum=$(sha256sum "$6" | cut -d' ' -f1)
   echo "PackageName: $1"; echo "SPDXID: SPDXRef-Upstream-$1"; echo "PackageVersion: $2"
   echo "PackageDownloadLocation: $4"; echo 'FilesAnalyzed: false'; echo "PackageChecksum: SHA256: $5"
   echo "ExternalRef: OTHER binary-sha256 $binsum"; echo "PackageLicenseConcluded: $3"; echo "PackageLicenseDeclared: $3"; echo
   echo "Relationship: SPDXRef-Package-receiver-image CONTAINS SPDXRef-Upstream-$1" >>"$rel"
 done
 cat "$rel"
 echo "Relationship: SPDXRef-Upstream-shairport-sync DEPENDS_ON $libavcodec_id"
 echo "Relationship: SPDXRef-Upstream-shairport-sync DEPENDS_ON $libavahi_id"
 echo "Relationship: SPDXRef-Upstream-nqptp DEPENDS_ON $libc_id"
} >"$tmp"
install -m0644 "$tmp" "$out"
