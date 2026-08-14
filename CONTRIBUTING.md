# Contributing

Open a focused change with a clear rationale and tests. Do not commit credentials, personal data, deployment inventory, non-example addresses, or generated build artifacts.

Run before submitting:

```sh
components/airplay-receiver/test.sh
(cd components/route-authority && cargo fmt --check && cargo clippy --all-targets --all-features --locked -- -D warnings && cargo test --all-features --locked)
SKIP_INTEGRATION=1 components/snapserver/scripts/test
```

Changes to container dependencies must keep versions and image digests pinned, update the relevant lock/inventory documentation, preserve upstream notices exactly, and explain license implications. Integration-affecting changes should also run the Snapserver and live route-authority tests when Docker is available.
