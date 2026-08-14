use async_trait::async_trait;
use household_audio_router::{
    AcquireRequest, AdapterPolicy, Authority, ReporterPolicy, RouteDriver, TargetConfig,
    snapcast::SnapcastDriver,
};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;
struct Gate {
    inner: Arc<SnapcastDriver>,
    available: AtomicBool,
}
#[async_trait]
impl RouteDriver for Gate {
    async fn set_route(&self, t: &str, r: &str) -> Result<(), String> {
        if !self.available.load(Ordering::Relaxed) {
            return Err("simulated uncertainty".into());
        }
        self.inner.set_route(t, r).await
    }
    async fn observe_route(&self, t: &str) -> Result<String, String> {
        if !self.available.load(Ordering::Relaxed) {
            return Err("simulated uncertainty".into());
        }
        self.inner.observe_route(t).await
    }
    async fn probe(&self) -> Result<(), String> {
        if !self.available.load(Ordering::Relaxed) {
            return Err("simulated uncertainty".into());
        }
        self.inner.probe().await
    }
}
#[tokio::test]
#[ignore = "requires tests/live-snapcast.sh"]
async fn real_snapcast_035_decision_switch_readback_drift_and_uncertainty() {
    let url = std::env::var("SNAPCAST_LIVE_URL").expect("live URL");
    let group = std::env::var("SNAPCAST_LIVE_GROUP").expect("live group");
    let real = Arc::new(SnapcastDriver::new(url));
    real.probe().await.unwrap();
    real.set_route(&group, "AirPlay").await.unwrap();
    assert_eq!(real.observe_route(&group).await.unwrap(), "AirPlay");
    real.set_route(&group, "idle").await.unwrap();
    assert_eq!(real.observe_route(&group).await.unwrap(), "idle");
    let gate = Arc::new(Gate {
        inner: real.clone(),
        available: AtomicBool::new(true),
    });
    let authority = Authority::new(
        vec![TargetConfig {
            name: "live".into(),
            idle_route: "idle".into(),
            driver_target: group.clone(),
        }],
        gate.clone(),
    )
    .unwrap();
    let initial = authority.reconcile_target("live").await.unwrap();
    assert!(initial.reconciled);
    let reporter = ReporterPolicy {
        reporter: "live-reporter".into(),
        targets: HashSet::from(["live".into()]),
    };
    authority.presence("live", &reporter, true).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let adapter = AdapterPolicy {
        source: "live-adapter".into(),
        priority: 10,
        targets: HashSet::from(["live".into()]),
        routes: HashSet::from(["AirPlay".into()]),
        min_ttl_ms: 10,
        max_ttl_ms: 10_000,
        resume_preempted: false,
    };
    gate.available.store(false, Ordering::Relaxed);
    let req = AcquireRequest {
        lease_id: Uuid::new_v4(),
        idempotency_key: "live-uncertain".into(),
        route: "AirPlay".into(),
        ttl_ms: 5_000,
    };
    let accepted = authority
        .acquire("live", &adapter, req.clone())
        .await
        .unwrap();
    assert_eq!(accepted.state.desired_route, "AirPlay");
    assert!(!accepted.state.reconciled);
    assert_eq!(
        authority.acquire("live", &adapter, req).await.unwrap(),
        accepted
    );
    gate.available.store(true, Ordering::Relaxed);
    let switched = authority.reconcile_target("live").await.unwrap();
    assert_eq!(switched.observed_route.as_deref(), Some("AirPlay"));
    assert!(switched.reconciled);
    real.set_route(&group, "MA-Test-PCM").await.unwrap();
    assert_eq!(real.observe_route(&group).await.unwrap(), "MA-Test-PCM");
    let healed = authority.reconcile_target("live").await.unwrap();
    assert_eq!(healed.observed_route.as_deref(), Some("AirPlay"));
    assert!(healed.reconciled);
}
