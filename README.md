# Household Audio Fabric

Household Audio Fabric is a reference implementation for receiving network audio, arbitrating which source owns an output target, and distributing synchronized PCM audio. It is organized as three independently buildable components:

- [`airplay-receiver`](components/airplay-receiver): a pinned Shairport Sync and NQPTP receiver that emits raw PCM and metadata.
- [`route-authority`](components/route-authority): a single-replica Rust lease authority that reconciles desired routes through Snapcast JSON-RPC.
- [`snapserver`](components/snapserver): a pinned Snapcast server image and an isolated integration-test client.

A typical deployment sends receiver PCM to a named Snapserver TCP stream. Authenticated adapters request time-bounded route leases; the authority applies policy and selects the corresponding Snapcast stream. Endpoints consume synchronized streams through Snapclients.

## Status

This repository is an engineering reference, not a turnkey production deployment. It publishes source code only; its CI builds images for validation but does not distribute them. Automated unit, contract, container, and live Snapcast tests cover the software boundary. Physical receiver discovery, playback, reconnection, realtime scheduling under the chosen container runtime, and site-specific network policy remain deployment gates. No orchestration manifests or production credentials are included.

## Threat boundaries

Treat the audio LAN, host firewall, container runtime, bearer-token delivery, and Snapcast control plane as distinct trust boundaries. The receiver deliberately relies on host networking, host Avahi, narrow capabilities, and a host-installed nftables attestation. The authority authenticates callers but Snapcast itself provides neither authentication nor TLS. Keep all media and control ports on a trusted network, terminate transport security at an appropriate gateway if traffic crosses that boundary, run one authority replica, and supply unique secrets at runtime. See [SECURITY.md](SECURITY.md) and component documentation before deployment.

## Build and test

Requirements are Docker with Compose, Bash, Rust/Cargo, Python 3, and standard POSIX shell tools.

```sh
components/airplay-receiver/test.sh
(cd components/route-authority && cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-features --locked)
components/snapserver/scripts/test
components/route-authority/tests/live-snapcast.sh
```

Component READMEs document image builds and runtime contracts. All examples use reserved or placeholder identifiers; replace them only in private deployment configuration.

## Licensing and third-party engines

The repository's original code is available under the [MIT License](LICENSE). That license does **not** relicense third-party software, copied notices, dependency packages, or built images. The images incorporate separately licensed engines and libraries, including GPL/LGPL-covered software. Distributors must retain upstream notices and meet the applicable source and redistribution obligations. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), component lock files, and image-resident license/package inventories.
