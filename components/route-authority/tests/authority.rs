use async_trait::async_trait;
use household_audio_router::{
    AcquireRequest, AdapterPolicy, Authority, AuthorityError, ReleaseRequest, RenewRequest,
    ReporterPolicy, RouteDriver, TargetConfig,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;
use uuid::Uuid;
#[derive(Default)]
struct Fake {
    routes: Mutex<HashMap<String, String>>,
    reachable: AtomicBool,
}
impl Fake {
    fn new() -> Self {
        Self {
            routes: Mutex::new(HashMap::from([("group".into(), "idle".into())])),
            reachable: AtomicBool::new(true),
        }
    }
}
#[async_trait]
impl RouteDriver for Fake {
    async fn set_route(&self, t: &str, r: &str) -> Result<(), String> {
        if !self.reachable.load(Ordering::Relaxed) {
            return Err("down".into());
        }
        self.routes.lock().await.insert(t.into(), r.into());
        Ok(())
    }
    async fn observe_route(&self, t: &str) -> Result<String, String> {
        if !self.reachable.load(Ordering::Relaxed) {
            return Err("down".into());
        }
        self.routes
            .lock()
            .await
            .get(t)
            .cloned()
            .ok_or_else(|| "missing".into())
    }
    async fn probe(&self) -> Result<(), String> {
        if self.reachable.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err("down".into())
        }
    }
}
fn setup() -> (Authority, Arc<Fake>) {
    let driver = Arc::new(Fake::new());
    let a = Authority::new(
        vec![TargetConfig {
            name: "room".into(),
            idle_route: "idle".into(),
            driver_target: "group".into(),
        }],
        driver.clone(),
    )
    .unwrap();
    (a, driver)
}
fn adapter(source: &str, priority: i32, resume: bool) -> AdapterPolicy {
    AdapterPolicy {
        source: source.into(),
        priority,
        targets: HashSet::from(["room".into()]),
        routes: HashSet::from(["music".into(), "speech".into()]),
        min_ttl_ms: 10,
        max_ttl_ms: 1_000,
        resume_preempted: resume,
    }
}
fn acquire(route: &str, key: &str, ttl: u64) -> AcquireRequest {
    AcquireRequest {
        lease_id: Uuid::new_v4(),
        idempotency_key: key.into(),
        route: route.into(),
        ttl_ms: ttl,
    }
}
fn reporter() -> ReporterPolicy {
    ReporterPolicy {
        reporter: "presence".into(),
        targets: HashSet::from(["room".into()]),
    }
}
async fn eligible(a: &Authority, _p: &AdapterPolicy) {
    a.presence("room", &reporter(), true).await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
}
#[tokio::test(start_paused = true)]
async fn fixed_targets_unknown_get_never_allocates() {
    let (a, _) = setup();
    assert_eq!(a.snapshot_all().await.len(), 1);
    assert_eq!(
        a.snapshot("unknown").await.unwrap_err(),
        AuthorityError::UnknownTarget
    );
    assert_eq!(a.snapshot_all().await.len(), 1);
}
#[tokio::test(start_paused = true)]
async fn source_priority_and_acl_are_server_policy() {
    let (a, _) = setup();
    let low = adapter("airplay", 10, false);
    eligible(&a, &low).await;
    let first = a
        .acquire("room", &low, acquire("music", "one", 100))
        .await
        .unwrap();
    let equal = adapter("attacker-name", 10, false);
    assert_eq!(
        a.acquire("room", &equal, acquire("speech", "two", 100))
            .await
            .unwrap_err(),
        AuthorityError::Contended
    );
    let forbidden = AdapterPolicy {
        routes: HashSet::from(["speech".into()]),
        ..adapter("restricted", 20, false)
    };
    assert_eq!(
        a.acquire("room", &forbidden, acquire("music", "three", 100))
            .await
            .unwrap_err(),
        AuthorityError::Forbidden
    );
    assert_eq!(first.state.lease.unwrap().source, "airplay");
}
#[tokio::test(start_paused = true)]
async fn restoration_is_policy_controlled_and_monotonic_valid() {
    let (a, _) = setup();
    let low = adapter("resumable", 1, true);
    let high = adapter("alarm", 9, false);
    eligible(&a, &low).await;
    let old = a
        .acquire("room", &low, acquire("music", "a", 200))
        .await
        .unwrap();
    let top = a
        .acquire("room", &high, acquire("speech", "b", 100))
        .await
        .unwrap();
    assert_eq!(
        top.preempted_lease_id,
        Some(old.state.lease.unwrap().lease_id)
    );
    let released = a
        .release(
            "room",
            &high,
            ReleaseRequest {
                lease_id: top.state.lease.as_ref().unwrap().lease_id,
                token: top.token,
                idempotency_key: "r".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        released.restored_lease_id,
        released.state.lease.as_ref().map(|l| l.lease_id)
    );
    assert_eq!(released.state.desired_route, "music");
    tokio::time::advance(std::time::Duration::from_millis(201)).await;
    a.tick().await;
    assert_eq!(a.snapshot("room").await.unwrap().desired_route, "idle");
}
#[tokio::test(start_paused = true)]
async fn airplay_default_does_not_auto_resume() {
    let (a, _) = setup();
    let airplay = adapter("airplay-over-ma", 1, false);
    let high = adapter("alarm", 9, false);
    eligible(&a, &airplay).await;
    let _old = a
        .acquire("room", &airplay, acquire("music", "a", 200))
        .await
        .unwrap();
    let top = a
        .acquire("room", &high, acquire("speech", "b", 100))
        .await
        .unwrap();
    assert_eq!(top.state.suspended_leases, 1);
    let out = a
        .release(
            "room",
            &high,
            ReleaseRequest {
                lease_id: top.state.lease.unwrap().lease_id,
                token: top.token,
                idempotency_key: "r".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out.restored_lease_id, None);
    assert_eq!(out.state.desired_route, "idle");
}
#[tokio::test(start_paused = true)]
async fn observed_state_comes_only_from_driver_and_read_reconciliation_emits() {
    let (a, driver) = setup();
    let p = adapter("x", 1, false);
    eligible(&a, &p).await;
    driver
        .routes
        .lock()
        .await
        .insert("group".into(), "external".into());
    let mut events = a.subscribe();
    let before = a.snapshot("room").await.unwrap();
    assert_eq!(before.observed_route, None, "GET performs no driver I/O");
    let healed = a.reconcile_target("room").await.unwrap();
    assert_eq!(healed.observed_route.as_deref(), Some("idle"));
    assert!(healed.reconciled);
    assert_eq!(events.try_recv().unwrap().kind, "observed");
}
#[tokio::test(start_paused = true)]
async fn endpoint_presence_has_five_and_thirty_second_semantics() {
    let (a, _) = setup();
    let p = adapter("airplay", 1, false);
    a.presence("room", &reporter(), true).await.unwrap();
    assert_eq!(
        a.acquire("room", &p, acquire("music", "early", 100))
            .await
            .unwrap_err(),
        AuthorityError::EndpointUnavailable
    );
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    let g = a
        .acquire("room", &p, acquire("music", "ok", 100))
        .await
        .unwrap();
    let offline = a.presence("room", &reporter(), false).await.unwrap();
    assert!(offline.lease.is_none());
    assert!(!offline.session_eligible);
    assert!(offline.advertisement_eligible);
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    assert!(!a.snapshot("room").await.unwrap().advertisement_eligible);
    let renew = RenewRequest {
        lease_id: g.state.lease.unwrap().lease_id,
        token: g.token,
        idempotency_key: "renew".into(),
        ttl_ms: 100,
    };
    assert_eq!(
        a.renew("room", &p, renew).await.unwrap_err(),
        AuthorityError::NotFound
    );
}
#[tokio::test(start_paused = true)]
async fn idempotency_auth_and_ttl_are_enforced() {
    let (a, _) = setup();
    let p = adapter("x", 1, false);
    eligible(&a, &p).await;
    let req = acquire("music", "same", 100);
    let one = a.acquire("room", &p, req.clone()).await.unwrap();
    assert_eq!(one, a.acquire("room", &p, req).await.unwrap());
    let bad = RenewRequest {
        lease_id: one.state.lease.unwrap().lease_id,
        token: Uuid::new_v4(),
        idempotency_key: "renew".into(),
        ttl_ms: 100,
    };
    assert_eq!(
        a.renew("room", &p, bad).await.unwrap_err(),
        AuthorityError::BadToken
    );
    assert!(matches!(
        a.acquire("room", &p, acquire("music", "ttl", 9)).await,
        Err(AuthorityError::Invalid(_))
    ));
}
#[tokio::test(start_paused = true)]
async fn readiness_requires_reachable_reconciled_driver() {
    let (a, driver) = setup();
    a.tick().await;
    assert!(a.is_ready());
    driver.reachable.store(false, Ordering::Relaxed);
    a.tick().await;
    assert!(!a.is_ready());
}

#[tokio::test(start_paused = true)]
async fn decision_commits_before_driver_failure_and_replay_precedes_eligibility() {
    let (a, driver) = setup();
    let p = adapter("source", 1, false);
    eligible(&a, &p).await;
    driver.reachable.store(false, Ordering::Relaxed);
    let req = acquire("music", "uncertain", 100);
    let accepted = a.acquire("room", &p, req.clone()).await.unwrap();
    assert_eq!(accepted.state.desired_route, "music");
    assert!(!accepted.state.reconciled);
    a.presence("room", &reporter(), false).await.unwrap();
    assert_eq!(
        a.acquire("room", &p, req).await.unwrap(),
        accepted,
        "validated cached replay wins before changed eligibility/clock/driver work"
    );
}
#[tokio::test(start_paused = true)]
async fn idempotency_scope_includes_principal_and_operation() {
    let (a, _) = setup();
    let low = adapter("one", 1, false);
    let high = adapter("two", 2, false);
    eligible(&a, &low).await;
    a.acquire("room", &low, acquire("music", "shared", 100))
        .await
        .unwrap();
    let second = a
        .acquire("room", &high, acquire("speech", "shared", 100))
        .await
        .unwrap();
    assert_eq!(second.state.lease.unwrap().source, "two");
}

struct BlockingDriver {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl RouteDriver for BlockingDriver {
    async fn set_route(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn observe_route(&self, _: &str) -> Result<String, String> {
        self.started.notify_one();
        self.release.notified().await;
        Ok("idle".into())
    }
    async fn probe(&self) -> Result<(), String> {
        Ok(())
    }
}
#[tokio::test(start_paused = true)]
async fn network_reconciliation_never_holds_state_mutex_and_stale_result_is_discarded() {
    let driver = Arc::new(BlockingDriver {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let a = Authority::new(
        vec![TargetConfig {
            name: "room".into(),
            idle_route: "idle".into(),
            driver_target: "group".into(),
        }],
        driver.clone(),
    )
    .unwrap();
    let p = adapter("source", 1, false);
    eligible(&a, &p).await;
    let worker = a.clone();
    let task = tokio::spawn(async move { worker.reconcile_target("room").await.unwrap() });
    driver.started.notified().await;
    let accepted = tokio::time::timeout(
        std::time::Duration::from_millis(10),
        a.acquire("room", &p, acquire("music", "during-io", 100)),
    )
    .await
    .expect("state mutex is not held across driver await")
    .unwrap();
    driver.release.notify_one();
    let result = task.await.unwrap();
    assert_eq!(accepted.state.desired_route, "music");
    assert_eq!(
        result.observed_route, None,
        "observation from an older version is discarded"
    );
}

#[tokio::test(start_paused = true)]
async fn committed_changes_synchronously_clear_readiness() {
    let (a, _) = setup();
    let p = adapter("source", 1, false);
    a.tick().await;
    assert!(a.is_ready());
    a.presence("room", &reporter(), true).await.unwrap();
    assert!(!a.is_ready());
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    a.tick().await;
    assert!(a.is_ready());
    a.acquire("room", &p, acquire("music", "ready", 100))
        .await
        .unwrap();
    assert!(!a.is_ready());
    a.tick().await;
    assert!(a.is_ready());
    a.presence("room", &reporter(), false).await.unwrap();
    assert!(!a.is_ready());
}
#[tokio::test(start_paused = true)]
async fn acquire_consuming_expiry_emits_expired_restored_then_acquired() {
    let (a, _) = setup();
    let low = adapter("low", 1, true);
    let high = adapter("high", 9, false);
    let newest = adapter("new", 10, false);
    eligible(&a, &low).await;
    a.acquire("room", &low, acquire("music", "low", 100))
        .await
        .unwrap();
    a.acquire("room", &high, acquire("speech", "high", 10))
        .await
        .unwrap();
    let mut events = a.subscribe();
    tokio::time::advance(std::time::Duration::from_millis(11)).await;
    a.acquire("room", &newest, acquire("speech", "new", 100))
        .await
        .unwrap();
    assert_eq!(events.try_recv().unwrap().kind, "expired");
    assert_eq!(events.try_recv().unwrap().kind, "restored");
    assert_eq!(events.try_recv().unwrap().kind, "acquired");
}
