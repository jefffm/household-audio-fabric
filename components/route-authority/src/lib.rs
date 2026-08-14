//! Serialized monotonic route authority. Decisions commit before external I/O.
pub mod api;
pub mod snapcast;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, broadcast},
    time::Instant,
};
use uuid::Uuid;
const ONLINE_ELIGIBILITY: Duration = Duration::from_secs(5);
const ADVERTISEMENT_WITHDRAW: Duration = Duration::from_secs(30);
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub name: String,
    pub idle_route: String,
    pub driver_target: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPolicy {
    pub source: String,
    pub priority: i32,
    pub targets: HashSet<String>,
    pub routes: HashSet<String>,
    pub min_ttl_ms: u64,
    pub max_ttl_ms: u64,
    #[serde(default)]
    pub resume_preempted: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReporterPolicy {
    pub reporter: String,
    pub targets: HashSet<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcquireRequest {
    pub lease_id: Uuid,
    pub idempotency_key: String,
    pub route: String,
    pub ttl_ms: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RenewRequest {
    pub lease_id: Uuid,
    pub token: Uuid,
    pub idempotency_key: String,
    pub ttl_ms: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRequest {
    pub lease_id: Uuid,
    pub token: Uuid,
    pub idempotency_key: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PresenceRequest {
    pub online: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseView {
    pub lease_id: Uuid,
    pub source: String,
    pub route: String,
    pub priority: i32,
    pub remaining_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub target: String,
    pub version: u64,
    pub desired_route: String,
    pub observed_route: Option<String>,
    pub reconciled: bool,
    pub endpoint_online: bool,
    pub session_eligible: bool,
    pub advertisement_eligible: bool,
    pub lease: Option<LeaseView>,
    pub suspended_leases: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    pub token: Uuid,
    pub preempted_lease_id: Option<Uuid>,
    pub state: TargetSnapshot,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseResult {
    pub released: bool,
    pub restored_lease_id: Option<Uuid>,
    pub state: TargetSnapshot,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityEvent {
    pub id: u64,
    pub kind: String,
    pub target: String,
    pub version: u64,
    pub state: TargetSnapshot,
}
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthorityError {
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("unknown target")]
    UnknownTarget,
    #[error("principal is not allowed for target, route, or capability")]
    Forbidden,
    #[error("endpoint is not continuously online and eligible")]
    EndpointUnavailable,
    #[error("target is held by an equal or higher priority lease")]
    Contended,
    #[error("lease not found or expired")]
    NotFound,
    #[error("lease token does not match")]
    BadToken,
    #[error("idempotency key was reused with different input")]
    IdempotencyConflict,
    #[error("route driver failed: {0}")]
    Driver(String),
}
#[async_trait]
pub trait RouteDriver: Send + Sync {
    async fn set_route(&self, driver_target: &str, route: &str) -> Result<(), String>;
    async fn observe_route(&self, driver_target: &str) -> Result<String, String>;
    async fn probe(&self) -> Result<(), String>;
}
#[derive(Clone, Debug)]
struct Lease {
    request: AcquireRequest,
    source: String,
    priority: i32,
    resume: bool,
    token: Uuid,
    deadline: Instant,
}
#[derive(Clone, Debug)]
enum CachedResponse {
    Grant(Grant),
    Release(ReleaseResult),
}
#[derive(Clone, Debug)]
struct Cached {
    fingerprint: String,
    response: CachedResponse,
}
#[derive(Debug)]
struct TargetState {
    cfg: TargetConfig,
    version: u64,
    desired: String,
    observed: Option<String>,
    lease: Option<Lease>,
    suspended: Vec<Lease>,
    online: bool,
    online_since: Option<Instant>,
    offline_since: Option<Instant>,
    cache: HashMap<String, Cached>,
}
struct TargetCell {
    state: Mutex<TargetState>,
    reconcile: Mutex<()>,
}
#[derive(Clone)]
pub struct Authority {
    targets: Arc<HashMap<String, Arc<TargetCell>>>,
    driver: Arc<dyn RouteDriver>,
    events: broadcast::Sender<AuthorityEvent>,
    next_event: Arc<AtomicU64>,
    ready: Arc<AtomicBool>,
}
impl Authority {
    pub fn new(
        configs: Vec<TargetConfig>,
        driver: Arc<dyn RouteDriver>,
    ) -> Result<Self, AuthorityError> {
        if configs.is_empty() {
            return Err(AuthorityError::Invalid(
                "at least one target is required".into(),
            ));
        }
        let mut targets = HashMap::new();
        for cfg in configs {
            if cfg.name.trim().is_empty()
                || cfg.idle_route.trim().is_empty()
                || cfg.driver_target.trim().is_empty()
                || targets.contains_key(&cfg.name)
            {
                return Err(AuthorityError::Invalid(
                    "targets must be unique and non-empty".into(),
                ));
            }
            let state = TargetState {
                desired: cfg.idle_route.clone(),
                cfg,
                version: 0,
                observed: None,
                lease: None,
                suspended: vec![],
                online: false,
                online_since: None,
                offline_since: None,
                cache: HashMap::new(),
            };
            targets.insert(
                state.cfg.name.clone(),
                Arc::new(TargetCell {
                    state: Mutex::new(state),
                    reconcile: Mutex::new(()),
                }),
            );
        }
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            targets: Arc::new(targets),
            driver,
            events,
            next_event: Arc::new(AtomicU64::new(1)),
            ready: Arc::new(AtomicBool::new(false)),
        })
    }
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<AuthorityEvent> {
        self.events.subscribe()
    }
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
    #[must_use]
    pub fn has_target(&self, target: &str) -> bool {
        self.targets.contains_key(target)
    }
    pub async fn acquire(
        &self,
        target: &str,
        a: &AdapterPolicy,
        r: AcquireRequest,
    ) -> Result<Grant, AuthorityError> {
        validate_acquire(a, target, &r)?;
        let cell = self.cell(target)?;
        let receipt = Instant::now();
        let mut s = cell.state.lock().await;
        let scope = scope(&a.source, "acquire", &r.idempotency_key);
        let fp = fingerprint(&(a.source.as_str(), "acquire", &r));
        if let Some(c) = s.cache.get(&scope) {
            return cached_grant(c, &fp);
        }
        let transition = advance(&mut s, Instant::now());
        self.emit_transition(&s, transition);
        if !session_eligible(&s, Instant::now()) {
            return Err(AuthorityError::EndpointUnavailable);
        }
        if let Some(active) = &s.lease {
            if active.request.lease_id == r.lease_id {
                return if active.request == r && active.source == a.source {
                    Ok(make_grant(&s, Instant::now(), active.token, None))
                } else {
                    Err(AuthorityError::IdempotencyConflict)
                };
            }
            if a.priority <= active.priority {
                return Err(AuthorityError::Contended);
            }
        }
        let old = s.lease.take();
        let preempted = old.as_ref().map(|l| l.request.lease_id);
        if let Some(l) = old {
            if l.deadline > Instant::now() {
                s.suspended.push(l)
            }
        }
        let commit = Instant::now().max(receipt);
        let token = Uuid::new_v4();
        s.lease = Some(Lease {
            request: r.clone(),
            source: a.source.clone(),
            priority: a.priority,
            resume: a.resume_preempted,
            token,
            deadline: commit + Duration::from_millis(r.ttl_ms),
        });
        s.desired = r.route.clone();
        s.version += 1;
        self.ready.store(false, Ordering::Relaxed);
        let out = make_grant(&s, Instant::now(), token, preempted);
        remember(&mut s, scope, fp, CachedResponse::Grant(out.clone()));
        self.emit("acquired", &s, Instant::now());
        Ok(out)
    }
    pub async fn renew(
        &self,
        target: &str,
        a: &AdapterPolicy,
        r: RenewRequest,
    ) -> Result<Grant, AuthorityError> {
        validate_mutation(a, target, &r.idempotency_key, r.ttl_ms)?;
        let cell = self.cell(target)?;
        let mut s = cell.state.lock().await;
        let key = scope(&a.source, "renew", &r.idempotency_key);
        let fp = fingerprint(&(a.source.as_str(), "renew", &r));
        if let Some(c) = s.cache.get(&key) {
            return cached_grant(c, &fp);
        }
        let transition = advance(&mut s, Instant::now());
        self.emit_transition(&s, transition);
        let lease = s.lease.as_mut().ok_or(AuthorityError::NotFound)?;
        if lease.source != a.source || lease.request.lease_id != r.lease_id {
            return Err(AuthorityError::NotFound);
        }
        if lease.token != r.token {
            return Err(AuthorityError::BadToken);
        }
        lease.deadline = Instant::now() + Duration::from_millis(r.ttl_ms);
        s.version += 1;
        let out = make_grant(&s, Instant::now(), r.token, None);
        remember(&mut s, key, fp, CachedResponse::Grant(out.clone()));
        self.emit("renewed", &s, Instant::now());
        Ok(out)
    }
    pub async fn release(
        &self,
        target: &str,
        a: &AdapterPolicy,
        r: ReleaseRequest,
    ) -> Result<ReleaseResult, AuthorityError> {
        validate_key_target(a, target, &r.idempotency_key)?;
        let cell = self.cell(target)?;
        let mut s = cell.state.lock().await;
        let key = scope(&a.source, "release", &r.idempotency_key);
        let fp = fingerprint(&(a.source.as_str(), "release", &r));
        if let Some(c) = s.cache.get(&key) {
            return cached_release(c, &fp);
        }
        let transition = advance(&mut s, Instant::now());
        self.emit_transition(&s, transition);
        let lease = s.lease.as_ref().ok_or(AuthorityError::NotFound)?;
        if lease.source != a.source || lease.request.lease_id != r.lease_id {
            return Err(AuthorityError::NotFound);
        }
        if lease.token != r.token {
            return Err(AuthorityError::BadToken);
        }
        s.lease = None;
        let restored = restore_or_idle(&mut s, Instant::now());
        s.version += 1;
        self.ready.store(false, Ordering::Relaxed);
        let out = ReleaseResult {
            released: true,
            restored_lease_id: restored,
            state: snapshot(&s, Instant::now()),
        };
        remember(&mut s, key, fp, CachedResponse::Release(out.clone()));
        self.emit("released", &s, Instant::now());
        Ok(out)
    }
    pub async fn presence(
        &self,
        target: &str,
        reporter: &ReporterPolicy,
        online: bool,
    ) -> Result<TargetSnapshot, AuthorityError> {
        if !reporter.targets.contains(target) {
            return Err(AuthorityError::Forbidden);
        }
        let cell = self.cell(target)?;
        let mut s = cell.state.lock().await;
        let now = Instant::now();
        if online == s.online {
            return Ok(snapshot(&s, Instant::now()));
        }
        if online {
            s.online = true;
            s.online_since = Some(now);
            s.offline_since = None;
        } else {
            s.online = false;
            s.online_since = None;
            s.offline_since = Some(now);
            if s.lease.take().is_some() {
                s.suspended.clear();
                s.desired = s.cfg.idle_route.clone();
            }
        }
        s.version += 1;
        self.ready.store(false, Ordering::Relaxed);
        let out = snapshot(&s, Instant::now());
        self.emit("presence", &s, Instant::now());
        Ok(out)
    }
    pub async fn snapshot(&self, target: &str) -> Result<TargetSnapshot, AuthorityError> {
        let cell = self.cell(target)?;
        let mut s = cell.state.lock().await;
        let transition = advance(&mut s, Instant::now());
        self.emit_transition(&s, transition);
        Ok(snapshot(&s, Instant::now()))
    }
    pub async fn snapshot_all(&self) -> Vec<TargetSnapshot> {
        let mut out = Vec::new();
        for (name, cell) in self.targets.iter() {
            let mut s = cell.state.lock().await;
            let transition = advance(&mut s, Instant::now());
            self.emit_transition(&s, transition);
            out.push((name.clone(), snapshot(&s, Instant::now())))
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out.into_iter().map(|x| x.1).collect()
    }
    pub async fn reconcile_target(&self, target: &str) -> Result<TargetSnapshot, AuthorityError> {
        let cell = self.cell(target)?;
        let _serial = cell.reconcile.lock().await;
        let (version, desired, driver_target) = {
            let mut s = cell.state.lock().await;
            let transition = advance(&mut s, Instant::now());
            self.emit_transition(&s, transition);
            (s.version, s.desired.clone(), s.cfg.driver_target.clone())
        };
        let mut observed = self
            .driver
            .observe_route(&driver_target)
            .await
            .map_err(AuthorityError::Driver)?;
        if observed != desired {
            self.driver
                .set_route(&driver_target, &desired)
                .await
                .map_err(AuthorityError::Driver)?;
            observed = self
                .driver
                .observe_route(&driver_target)
                .await
                .map_err(AuthorityError::Driver)?;
        }
        let mut s = cell.state.lock().await;
        let version_matches = s.version == version;
        if version_matches && s.observed.as_ref() != Some(&observed) {
            s.observed = Some(observed);
            s.version += 1;
            self.emit("observed", &s, Instant::now())
        }
        let mut out = snapshot(&s, Instant::now());
        if !version_matches {
            out.reconciled = false;
        }
        Ok(out)
    }
    pub async fn tick(&self) {
        let mut ok = self.driver.probe().await.is_ok();
        for name in self.targets.keys() {
            match self.reconcile_target(name).await {
                Ok(s) if s.reconciled => {}
                _ => ok = false,
            }
        }
        self.ready.store(ok, Ordering::Relaxed)
    }
    fn cell(&self, target: &str) -> Result<Arc<TargetCell>, AuthorityError> {
        self.targets
            .get(target)
            .cloned()
            .ok_or(AuthorityError::UnknownTarget)
    }
    fn emit_transition(&self, s: &TargetState, transition: Option<AdvanceOutcome>) {
        if let Some(outcome) = transition {
            self.ready.store(false, Ordering::Relaxed);
            self.emit("expired", s, Instant::now());
            if outcome.restored {
                self.emit("restored", s, Instant::now());
            }
        }
    }
    fn emit(&self, kind: &str, s: &TargetState, now: Instant) {
        let id = self.next_event.fetch_add(1, Ordering::Relaxed);
        let _ = self.events.send(AuthorityEvent {
            id,
            kind: kind.into(),
            target: s.cfg.name.clone(),
            version: s.version,
            state: snapshot(s, now),
        });
    }
}
#[derive(Clone, Copy)]
struct AdvanceOutcome {
    restored: bool,
}
fn advance(s: &mut TargetState, now: Instant) -> Option<AdvanceOutcome> {
    if s.lease.as_ref().is_some_and(|l| l.deadline <= now) {
        s.lease = None;
        let restored = restore_or_idle(s, now).is_some();
        s.version += 1;
        Some(AdvanceOutcome { restored })
    } else {
        None
    }
}
fn restore_or_idle(s: &mut TargetState, now: Instant) -> Option<Uuid> {
    while let Some(l) = s.suspended.pop() {
        if l.resume && l.deadline > now {
            let id = l.request.lease_id;
            s.desired = l.request.route.clone();
            s.lease = Some(l);
            return Some(id);
        }
    }
    s.desired = s.cfg.idle_route.clone();
    None
}
fn session_eligible(s: &TargetState, now: Instant) -> bool {
    s.online
        && s.online_since
            .is_some_and(|t| now.duration_since(t) >= ONLINE_ELIGIBILITY)
}
fn advertisement_eligible(s: &TargetState, now: Instant) -> bool {
    if s.online {
        return session_eligible(s, now);
    }
    s.offline_since
        .is_some_and(|t| now.duration_since(t) < ADVERTISEMENT_WITHDRAW)
}
fn snapshot(s: &TargetState, now: Instant) -> TargetSnapshot {
    TargetSnapshot {
        target: s.cfg.name.clone(),
        version: s.version,
        desired_route: s.desired.clone(),
        observed_route: s.observed.clone(),
        reconciled: s.observed.as_ref() == Some(&s.desired),
        endpoint_online: s.online,
        session_eligible: session_eligible(s, now),
        advertisement_eligible: advertisement_eligible(s, now),
        lease: s.lease.as_ref().map(|l| LeaseView {
            lease_id: l.request.lease_id,
            source: l.source.clone(),
            route: l.request.route.clone(),
            priority: l.priority,
            remaining_ms: l
                .deadline
                .saturating_duration_since(now)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
        }),
        suspended_leases: s.suspended.len(),
    }
}
fn validate_acquire(a: &AdapterPolicy, t: &str, r: &AcquireRequest) -> Result<(), AuthorityError> {
    validate_mutation(a, t, &r.idempotency_key, r.ttl_ms)?;
    if !a.routes.contains(&r.route) {
        return Err(AuthorityError::Forbidden);
    }
    Ok(())
}
fn validate_mutation(a: &AdapterPolicy, t: &str, k: &str, ttl: u64) -> Result<(), AuthorityError> {
    validate_key_target(a, t, k)?;
    if !(a.min_ttl_ms..=a.max_ttl_ms).contains(&ttl) {
        return Err(AuthorityError::Invalid(format!(
            "ttl_ms must be in {}..={}",
            a.min_ttl_ms, a.max_ttl_ms
        )));
    }
    Ok(())
}
fn validate_key_target(a: &AdapterPolicy, t: &str, k: &str) -> Result<(), AuthorityError> {
    if !a.targets.contains(t) {
        return Err(AuthorityError::Forbidden);
    }
    if k.trim().is_empty() || k.len() > 128 {
        return Err(AuthorityError::Invalid(
            "idempotency_key must contain 1..128 characters".into(),
        ));
    }
    Ok(())
}
fn scope(principal: &str, op: &str, key: &str) -> String {
    format!("{principal}\0{op}\0{key}")
}
fn fingerprint<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("request serialization is infallible")
}
fn cached_grant(c: &Cached, fp: &str) -> Result<Grant, AuthorityError> {
    if c.fingerprint == fp {
        if let CachedResponse::Grant(v) = &c.response {
            return Ok(v.clone());
        }
    }
    Err(AuthorityError::IdempotencyConflict)
}
fn cached_release(c: &Cached, fp: &str) -> Result<ReleaseResult, AuthorityError> {
    if c.fingerprint == fp {
        if let CachedResponse::Release(v) = &c.response {
            return Ok(v.clone());
        }
    }
    Err(AuthorityError::IdempotencyConflict)
}
fn remember(s: &mut TargetState, k: String, fp: String, response: CachedResponse) {
    if s.cache.len() >= 256 {
        if let Some(k) = s.cache.keys().next().cloned() {
            s.cache.remove(&k);
        }
    }
    s.cache.insert(
        k,
        Cached {
            fingerprint: fp,
            response,
        },
    );
}
fn make_grant(s: &TargetState, now: Instant, token: Uuid, preempted: Option<Uuid>) -> Grant {
    Grant {
        token,
        preempted_lease_id: preempted,
        state: snapshot(s, now),
    }
}
