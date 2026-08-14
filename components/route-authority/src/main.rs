use household_audio_router::{
    AdapterPolicy, Authority, ReporterPolicy, TargetConfig,
    api::{ApiState, AuthConfig, router},
    snapcast::SnapcastDriver,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    targets: Vec<TargetConfig>,
    adapters: Vec<AdapterSecret>,
    reporters: Vec<ReporterSecret>,
    reader_tokens: Vec<String>,
    admin_tokens: Vec<String>,
    snapcast_url: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterSecret {
    token: String,
    policy: AdapterPolicy,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReporterSecret {
    token: String,
    policy: ReporterPolicy,
}
fn validate(cfg: Config) -> Result<(Vec<TargetConfig>, AuthConfig, String), String> {
    let url =
        reqwest::Url::parse(&cfg.snapcast_url).map_err(|e| format!("invalid snapcast_url: {e}"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("snapcast_url must be an http(s) URL without credentials".into());
    }
    let names: HashSet<_> = cfg.targets.iter().map(|t| t.name.as_str()).collect();
    let groups: HashSet<_> = cfg
        .targets
        .iter()
        .map(|t| t.driver_target.as_str())
        .collect();
    if cfg.targets.iter().any(|t| {
        t.name.trim().is_empty()
            || t.idle_route.trim().is_empty()
            || t.driver_target.trim().is_empty()
    }) || cfg.targets.is_empty()
        || names.len() != cfg.targets.len()
        || groups.len() != cfg.targets.len()
    {
        return Err("targets and driver_target groups must be nonempty and unique".into());
    }
    let mut used = HashSet::new();
    let mut sources = HashSet::new();
    let mut adapters = HashMap::new();
    for item in cfg.adapters {
        let p = &item.policy;
        if item.token.is_empty()
            || p.source.trim().is_empty()
            || !sources.insert(p.source.clone())
            || p.routes.is_empty()
            || p.targets.is_empty()
            || p.min_ttl_ms > p.max_ttl_ms
            || p.targets.iter().any(|t| !names.contains(t.as_str()))
            || p.routes.iter().any(|r| r.trim().is_empty())
            || !used.insert(item.token.clone())
            || adapters.insert(item.token, item.policy).is_some()
        {
            return Err("invalid adapter credential/policy/ACL/TTL".into());
        }
    }
    let mut reporters = HashMap::new();
    let mut reporter_names = HashSet::new();
    for item in cfg.reporters {
        let p = &item.policy;
        if item.token.is_empty()
            || p.reporter.trim().is_empty()
            || !reporter_names.insert(p.reporter.clone())
            || p.targets.is_empty()
            || p.targets.iter().any(|t| !names.contains(t.as_str()))
            || !used.insert(item.token.clone())
            || reporters.insert(item.token, item.policy).is_some()
        {
            return Err("invalid reporter credential/policy/ACL".into());
        }
    }
    let mut reporter_assignments: HashMap<&str, usize> = HashMap::new();
    for policy in reporters.values() {
        for target in &policy.targets {
            *reporter_assignments.entry(target.as_str()).or_default() += 1;
        }
    }
    if names
        .iter()
        .any(|target| reporter_assignments.get(target).copied() != Some(1))
    {
        return Err("every target must have exactly one authoritative endpoint reporter".into());
    }
    let reader_count = cfg.reader_tokens.len();
    let admin_count = cfg.admin_tokens.len();
    let readers: HashSet<_> = cfg.reader_tokens.into_iter().collect();
    let admins: HashSet<_> = cfg.admin_tokens.into_iter().collect();
    if readers.len() != reader_count
        || admins.len() != admin_count
        || adapters.is_empty()
        || reporters.is_empty()
        || readers.is_empty()
        || admins.is_empty()
        || readers
            .iter()
            .chain(admins.iter())
            .any(|t| t.is_empty() || !used.insert(t.clone()))
    {
        return Err("all credential roles must be nonempty and mutually disjoint".into());
    }
    Ok((
        cfg.targets,
        AuthConfig {
            adapters,
            reporters,
            readers,
            admins,
        },
        cfg.snapcast_url,
    ))
}
fn port() -> Result<u16, String> {
    match env::var("PORT") {
        Ok(v) => v.parse().map_err(|_| "PORT must be a valid u16".into()),
        Err(env::VarError::NotPresent) => Ok(8080),
        Err(e) => Err(e.to_string()),
    }
}
#[tokio::main]
async fn main() {
    let port = port().expect("invalid PORT");
    let raw = env::var("ROUTER_CONFIG_JSON").expect("ROUTER_CONFIG_JSON is required (fail-closed)");
    let cfg: Config =
        serde_json::from_str(&raw).expect("ROUTER_CONFIG_JSON must match the documented schema");
    let (targets, auth, url) = validate(cfg).expect("invalid ROUTER_CONFIG_JSON policy");
    let authority =
        Authority::new(targets, Arc::new(SnapcastDriver::new(url))).expect("invalid targets");
    let state = ApiState::new(authority.clone(), auth);
    let mut worker_shutdown = state.subscribe_shutdown();
    let worker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = worker_shutdown.recv() => break,
                _ = interval.tick() => authority.tick().await,
            }
        }
    });
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))
            .await
            .expect("bind listener");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let shutdown_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state.clone()))
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
    });
    wait_signal().await;
    // Stop acceptance, SSE streams, reconciliation and in-flight requests together.
    // A hard deadline bounds malicious partial headers/bodies and stuck peers.
    shutdown_state.shutdown();
    let _ = stop_tx.send(());
    // The worker shares the same cancellation broadcast.
    // SIGTERM cancellation is sent by aborting the worker below as a final backstop.
    worker.abort();
    let mut server = server;
    if tokio::time::timeout(Duration::from_secs(3), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
}
async fn wait_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install ctrl-c handler")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {()=ctrl_c=>{},()=terminate=>{}}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_config_rejects_unknown_fields_bad_url_and_cross_role_tokens() {
        let unknown = r#"{"targets":[],"adapters":[],"reporters":[],"reader_tokens":[],"admin_tokens":[],"snapcast_url":"file:///tmp/x","extra":1}"#;
        assert!(serde_json::from_str::<Config>(unknown).is_err());
        let bad_url = r#"{"targets":[],"adapters":[],"reporters":[],"reader_tokens":[],"admin_tokens":[],"snapcast_url":"file:///tmp/x"}"#;
        assert!(validate(serde_json::from_str(bad_url).unwrap()).is_err());
        let duplicate = r#"{"targets":[{"name":"room","idle_route":"idle","driver_target":"group"}],"adapters":[{"token":"same","policy":{"source":"source","priority":1,"targets":["room"],"routes":["music"],"min_ttl_ms":1,"max_ttl_ms":2}}],"reporters":[{"token":"reporter","policy":{"reporter":"presence","targets":["room"]}}],"reader_tokens":["same"],"admin_tokens":["admin"],"snapcast_url":"http://snapserver:1780/jsonrpc"}"#;
        assert!(validate(serde_json::from_str(duplicate).unwrap()).is_err());
        let reporters = r#"{"targets":[{"name":"room","idle_route":"idle","driver_target":"group"}],"adapters":[{"token":"adapter","policy":{"source":"source","priority":1,"targets":["room"],"routes":["music"],"min_ttl_ms":1,"max_ttl_ms":2}}],"reporters":[{"token":"r1","policy":{"reporter":"one","targets":["room"]}},{"token":"r2","policy":{"reporter":"two","targets":["room"]}}],"reader_tokens":["read"],"admin_tokens":["admin"],"snapcast_url":"http://snapserver:1780/jsonrpc"}"#;
        assert!(validate(serde_json::from_str(reporters).unwrap()).is_err());
    }
}
