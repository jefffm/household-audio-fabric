# Third-party notices

The root MIT license applies only to original repository code. It does not relicense dependencies, upstream engines, package metadata, or their notices.

## AirPlay receiver image

The image builds pinned Shairport Sync 5.2.1 and NQPTP 1.2.8 sources. Their exact upstream license files are copied unmodified from the verified source archives into `/usr/share/doc/shairport-sync/` and `/usr/share/doc/nqptp/` during the image build. Shairport Sync declares MIT for its own code and carries additional notices; NQPTP is GPL-2.0-only. Debian runtime packages include separately licensed material, including GPL/LGPL-covered FFmpeg libraries. `components/airplay-receiver/SOURCES.lock` preserves source URLs and hashes; `LICENSES.md` documents notice locations; `generate-package-inventory.sh` emits the architecture-specific SPDX inventory without altering package metadata.

## Snapserver image

Snapcast 0.35.0 server and test-client packages are downloaded from the upstream release and SHA-256 verified per architecture. Snapcast is GPL-3.0-or-later. Its source and binary packages also contain separately licensed portions, including LGPL-2.1-or-later, Zlib, and Expat material; Debian dependencies retain their own licenses; exact direct-package versions are preserved in `components/snapserver/packages.lock`, and Debian copyright files remain in the image.

## Rust route authority

Rust dependency names, exact versions, checksums, and registry sources are preserved in `components/route-authority/Cargo.lock`. Each crate remains under its own declared license. Use `cargo metadata` or a dedicated license-audit tool when preparing a distribution.

A container image is a combined distribution of many separately licensed works. Do not describe an image as simply “MIT.” Before redistribution, inspect the complete architecture-specific package inventory and license files, preserve notices, and provide source or a source offer wherever the GPL/LGPL terms require it. This notice is informational and not legal advice.
