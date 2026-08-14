# Household Audio Route Authority

Single-replica, in-memory authority for household audio route ownership. It has a
finite, declaratively configured target set and one serialized state machine per
target. Client JSON never supplies source identity or priority: a fail-closed
bearer credential selects an immutable adapter policy (source, fixed priority,
target/route ACL, TTL range, and whether its lease may resume after preemption).
Reader and administrator credentials are separate.

## Runtime configuration

`ROUTER_CONFIG_JSON` is required; the process refuses to start without it. Example:

```json
{
  "snapcast_url": "http://snapserver:1780/jsonrpc",
  "targets": [{"name":"target-a","idle_route":"silence","driver_target":"group-a"}],
  "adapters": [{
    "token":"<adapter-token>",
    "policy": {
      "source":"receiver-adapter",
      "priority":20,
      "targets":["target-a"],
      "routes":["receiver-pcm"],
      "min_ttl_ms":1000,
      "max_ttl_ms":60000,
      "resume_preempted":false
    }
  }],
  "reporters": [{
    "token":"<reporter-token>",
    "policy":{"reporter":"endpoint-reporter","targets":["target-a"]}
  }],
  "reader_tokens":["<reader-token>"],
  "admin_tokens":["<admin-token>"]
}
```

Only `PORT` (default 8080) is configured separately. Secrets should come from a
Kubernetes Secret, not a ConfigMap or command line.

## Semantics and driver boundary

Lease deadlines use monotonic Tokio time. Strictly greater priority preempts.
A preempted lease is retained only when its adapter opts into resume; restoration
also requires its original monotonic deadline still to be valid. AirPlay-over-MA
should leave `resume_preempted` false. Release, expiration, or endpoint loss
selects the configured idle stream when no resumable lease remains.

`desired_route` is authority intent. `observed_route` is read back from Snapcast;
the service never calls a timer-copied value effective. The production driver
uses Snapcast 0.35 HTTP JSON-RPC: `Server.GetStatus`, idempotent
`Group.SetStream`, and subsequent observation, with connection pooling disabled
and bounded connect/total timeouts for Snapcast 0.35 compatibility. Authority
mutations commit and cache their deterministic accepted response before any
fallible driver call; the per-target worker heals desired/observed drift using a
two-phase version check without holding state locks across network I/O. Readiness
is false until Snapcast is reachable and every target is observed reconciled.

Exactly one separately credentialed endpoint reporter is declaratively assigned
to every target (never a source adapter) and reports presence at `PUT /api/v1/targets/:target/presence`. A session is eligible only after five continuous online seconds. Going offline
fails its current session immediately, while `advertisement_eligible` remains
true for 30 seconds to give an AirPlay advertisement adapter withdrawal grace.

## API and operations

Mutation routes are under `/api/v1/targets/:target`; snapshots are `/api/v1/state`
and `/api/v1/targets/:target`; SSE is `/api/v1/events`. SSE events carry IDs and
state versions; a slow consumer receives a `gap` event instructing it to refetch
the snapshot. Shutdown cancels streams and the worker before Axum drains, then hard-aborts the
whole server after a three-second deadline so partial headers/bodies cannot pin
the process. `/healthz` is process
liveness, `/readyz` is driver/reconciliation readiness, `/metrics` is admin-only,
and `/openapi.json` is reader-authorized. All error bodies have stable `code` and
`message` fields.

The authority intentionally has no database or cross-replica consensus. Restart
returns configured targets to idle intent, so deploy exactly one replica. The
256-entry idempotency cache per target and SSE buffer are process-local.

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
```

Build the non-root container image from the **repository root** because its build context includes this component path:

```sh
docker build -f components/route-authority/Containerfile -t household-audio-router:local .
```

## Live integration

`tests/live-snapcast.sh` starts the sibling pinned Snapcast 0.35 server/client,
discovers its real group, and proves raw status/set/status compatibility, accepted
acquisition during simulated RPC uncertainty, deterministic replay, readback,
startup convergence, and drift healing. It uses a unique Compose project and
always removes it on exit.
