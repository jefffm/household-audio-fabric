use crate::{
    AcquireRequest, AdapterPolicy, Authority, AuthorityError, PresenceRequest, ReleaseRequest,
    RenewRequest, ReporterPolicy,
};
use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post, put},
};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::broadcast;

#[derive(Clone, Default)]
pub struct AuthConfig {
    pub adapters: HashMap<String, AdapterPolicy>,
    pub reporters: HashMap<String, ReporterPolicy>,
    pub readers: HashSet<String>,
    pub admins: HashSet<String>,
}
#[derive(Default)]
pub struct Metrics {
    requests: AtomicU64,
    errors: AtomicU64,
    responses_2xx: AtomicU64,
    responses_4xx: AtomicU64,
    responses_5xx: AtomicU64,
    events_lagged: AtomicU64,
}
#[derive(Clone)]
pub struct ApiState {
    pub authority: Authority,
    pub auth: Arc<AuthConfig>,
    metrics: Arc<Metrics>,
    shutdown: broadcast::Sender<()>,
}
impl ApiState {
    pub fn new(authority: Authority, auth: AuthConfig) -> Self {
        let (shutdown, _) = broadcast::channel(8);
        Self {
            authority,
            auth: Arc::new(auth),
            metrics: Arc::default(),
            shutdown,
        }
    }
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(());
    }
    #[must_use]
    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown.subscribe()
    }
}
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics))
        .route("/openapi.json", get(openapi))
        .route("/api/v1/state", get(all_state))
        .route("/api/v1/events", get(events))
        .route("/api/v1/targets/{target}", get(target_state))
        .route("/api/v1/targets/{target}/presence", put(presence))
        .route(
            "/api/v1/targets/{target}/lease",
            post(acquire).put(renew).delete(release),
        )
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(middleware::from_fn_with_state(state.clone(), observe_http))
        .with_state(state)
}
async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "route not found")
}
async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "method not allowed",
    )
}
async fn observe_http(
    State(s): State<ApiState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    s.metrics.requests.fetch_add(1, Ordering::Relaxed);
    let response = next.run(request).await;
    match response.status().as_u16() / 100 {
        2 => {
            s.metrics.responses_2xx.fetch_add(1, Ordering::Relaxed);
        }
        4 => {
            s.metrics.responses_4xx.fetch_add(1, Ordering::Relaxed);
            s.metrics.errors.fetch_add(1, Ordering::Relaxed);
        }
        5 => {
            s.metrics.responses_5xx.fetch_add(1, Ordering::Relaxed);
            s.metrics.errors.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    response
}
#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}
async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
async fn ready(State(s): State<ApiState>) -> Response {
    if s.authority.is_ready() {
        (
            StatusCode::OK,
            Json(Health {
                status: "ready",
                version: env!("CARGO_PKG_VERSION"),
            }),
        )
            .into_response()
    } else {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            "Snapcast is unreachable or configured targets are not reconciled",
        )
        .into_response()
    }
}
async fn acquire(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(target): Path<String>,
    payload: Result<Json<AcquireRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<crate::Grant>), ApiError> {
    let adapter = adapter(&s, &headers)?;
    let req = json_payload(payload)?;
    s.authority
        .acquire(&target, adapter, req)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)))
        .map_err(Into::into)
}
async fn renew(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(target): Path<String>,
    payload: Result<Json<RenewRequest>, JsonRejection>,
) -> Result<Json<crate::Grant>, ApiError> {
    let adapter = adapter(&s, &headers)?;
    s.authority
        .renew(&target, adapter, json_payload(payload)?)
        .await
        .map(Json)
        .map_err(Into::into)
}
async fn release(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(target): Path<String>,
    payload: Result<Json<ReleaseRequest>, JsonRejection>,
) -> Result<Json<crate::ReleaseResult>, ApiError> {
    let adapter = adapter(&s, &headers)?;
    s.authority
        .release(&target, adapter, json_payload(payload)?)
        .await
        .map(Json)
        .map_err(Into::into)
}
async fn presence(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(target): Path<String>,
    payload: Result<Json<PresenceRequest>, JsonRejection>,
) -> Result<Json<crate::TargetSnapshot>, ApiError> {
    let reporter = reporter(&s, &headers)?;
    s.authority
        .presence(&target, reporter, json_payload(payload)?.online)
        .await
        .map(Json)
        .map_err(Into::into)
}
async fn target_state(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(target): Path<String>,
) -> Result<Json<crate::TargetSnapshot>, ApiError> {
    read_auth(&s, &headers)?;
    s.authority
        .snapshot(&target)
        .await
        .map(Json)
        .map_err(Into::into)
}
async fn all_state(
    State(s): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::TargetSnapshot>>, ApiError> {
    read_auth(&s, &headers)?;
    Ok(Json(s.authority.snapshot_all().await))
}
async fn metrics(State(s): State<ApiState>, headers: HeaderMap) -> Result<Response, ApiError> {
    admin_auth(&s, &headers)?;
    let targets = s.authority.snapshot_all().await.len();
    let body = format!(
        "# TYPE route_authority_http_requests_total counter\nroute_authority_http_requests_total {}\n# TYPE route_authority_http_errors_total counter\nroute_authority_http_errors_total {}\nroute_authority_http_responses_total{{class=\"2xx\"}} {}\nroute_authority_http_responses_total{{class=\"4xx\"}} {}\nroute_authority_http_responses_total{{class=\"5xx\"}} {}\nroute_authority_sse_lagged_total {}\nroute_authority_targets {}\n",
        s.metrics.requests.load(Ordering::Relaxed),
        s.metrics.errors.load(Ordering::Relaxed),
        s.metrics.responses_2xx.load(Ordering::Relaxed),
        s.metrics.responses_4xx.load(Ordering::Relaxed),
        s.metrics.responses_5xx.load(Ordering::Relaxed),
        s.metrics.events_lagged.load(Ordering::Relaxed),
        targets
    );
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response())
}
async fn openapi(
    State(s): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    read_auth(&s, &headers)?;
    Ok(Json(openapi_document()))
}
async fn events(
    State(s): State<ApiState>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    read_auth(&s, &headers)?;
    let mut rx = s.authority.subscribe();
    let mut shutdown = s.shutdown.subscribe();
    let metrics = s.metrics;
    let output = stream! {loop{tokio::select!{biased;_ = shutdown.recv()=>break,message=rx.recv()=>match message{Ok(event)=>{let kind=event.kind.clone();yield Ok(Event::default().id(event.id.to_string()).event(kind).json_data(event).expect("event serializes"));},Err(broadcast::error::RecvError::Lagged(n))=>{metrics.events_lagged.fetch_add(n,Ordering::Relaxed);yield Ok(Event::default().id(format!("gap-{n}")).event("gap").json_data(json!({"refetch":"/api/v1/state","missed":n})).expect("gap serializes"));},Err(broadcast::error::RecvError::Closed)=>break}}}};
    Ok(Sse::new(output).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "valid bearer token required",
            )
        })
}
fn adapter<'a>(s: &'a ApiState, h: &HeaderMap) -> Result<&'a AdapterPolicy, ApiError> {
    let token = bearer(h)?;
    s.auth.adapters.get(token).ok_or_else(|| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "valid adapter bearer token required",
        )
    })
}
fn reporter<'a>(s: &'a ApiState, h: &HeaderMap) -> Result<&'a ReporterPolicy, ApiError> {
    let token = bearer(h)?;
    s.auth.reporters.get(token).ok_or_else(|| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "valid endpoint-reporter bearer token required",
        )
    })
}
fn read_auth(s: &ApiState, h: &HeaderMap) -> Result<(), ApiError> {
    let token = bearer(h)?;
    if s.auth.readers.contains(token) || s.auth.admins.contains(token) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "read authorization required",
        ))
    }
}
fn admin_auth(s: &ApiState, h: &HeaderMap) -> Result<(), ApiError> {
    let token = bearer(h)?;
    if s.auth.admins.contains(token) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "admin authorization required",
        ))
    }
}
fn json_payload<T>(r: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    r.map(|Json(v)| v).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            "request body must be valid JSON matching the API schema",
        )
    })
}
#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}
impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: &str) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}
impl From<AuthorityError> for ApiError {
    fn from(e: AuthorityError) -> Self {
        let (status, code) = match e {
            AuthorityError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
            AuthorityError::UnknownTarget => (StatusCode::NOT_FOUND, "unknown_target"),
            AuthorityError::Forbidden | AuthorityError::BadToken => {
                (StatusCode::FORBIDDEN, "forbidden")
            }
            AuthorityError::EndpointUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, "endpoint_unavailable")
            }
            AuthorityError::Contended | AuthorityError::IdempotencyConflict => {
                (StatusCode::CONFLICT, "conflict")
            }
            AuthorityError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            AuthorityError::Driver(_) => (StatusCode::BAD_GATEWAY, "driver_error"),
        };
        Self {
            status,
            code,
            message: e.to_string(),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}
#[must_use]
pub fn openapi_document() -> serde_json::Value {
    let responses = |success: &str, schema: &str| {
        let mut map = serde_json::Map::new();
        map.insert(success.into(), json!({"description":"success","content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")}}}}));
        for (status, description) in [
            ("400", "invalid JSON/request"),
            ("401", "invalid bearer"),
            ("403", "policy denied"),
            ("404", "target/lease not found"),
            ("409", "contention/idempotency conflict"),
            ("502", "driver failure"),
            ("503", "endpoint/driver unavailable"),
        ] {
            map.insert(status.into(), json!({"description":description,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Error"}}}}));
        }
        serde_json::Value::Object(map)
    };
    let target = || json!({"name":"target","in":"path","required":true,"schema":{"type":"string"}});
    let read =
        |schema: &str| json!({"security":[{"bearerAuth":[]}],"responses":responses("200",schema)});
    let mutation = |schema: &str, success: &str, output: &str| json!({"security":[{"bearerAuth":[]}],"parameters":[target()],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")}}}},"responses":responses(success,output)});
    json!({
        "openapi":"3.1.0","info":{"title":"Household Audio Route Authority","version":"1.0.0"},
        "paths":{
            "/healthz":{"get":{"responses":{"200":{"description":"process is live","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Health"}}}}}}},
            "/readyz":{"get":{"responses":responses("200","Health")}},
            "/metrics":{"get":{"security":[{"bearerAuth":[]}],"responses":{"200":{"description":"Prometheus metrics","content":{"text/plain":{"schema":{"type":"string"}}}},"401":{"description":"invalid bearer","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Error"}}}},"403":{"description":"admin denied","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Error"}}}}}}},"/openapi.json":{"get":read("OpenApiDocument")},
            "/api/v1/state":{"get":read("TargetSnapshotList")},
            "/api/v1/events":{"get":{"security":[{"bearerAuth":[]}],"responses":{"200":{"description":"versioned events and gap notifications","content":{"text/event-stream":{"schema":{"type":"string"}}}},"401":{"description":"invalid bearer","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Error"}}}},"403":{"description":"read denied","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Error"}}}}}}},
            "/api/v1/targets/{target}":{"get":{"security":[{"bearerAuth":[]}],"parameters":[target()],"responses":responses("200","TargetSnapshot")}},
            "/api/v1/targets/{target}/lease":{"post":mutation("AcquireRequest","201","Grant"),"put":mutation("RenewRequest","200","Grant"),"delete":mutation("ReleaseRequest","200","ReleaseResult")},
            "/api/v1/targets/{target}/presence":{"put":mutation("PresenceRequest","200","TargetSnapshot")}
        },
        "components":{"securitySchemes":{"bearerAuth":{"type":"http","scheme":"bearer"}},"schemas":{
            "AcquireRequest":{"type":"object","additionalProperties":false,"required":["lease_id","idempotency_key","route","ttl_ms"],"properties":{"lease_id":{"type":"string","format":"uuid"},"idempotency_key":{"type":"string"},"route":{"type":"string"},"ttl_ms":{"type":"integer","format":"uint64"}}},
            "RenewRequest":{"type":"object","additionalProperties":false,"required":["lease_id","token","idempotency_key","ttl_ms"],"properties":{"lease_id":{"type":"string","format":"uuid"},"token":{"type":"string","format":"uuid"},"idempotency_key":{"type":"string"},"ttl_ms":{"type":"integer","format":"uint64"}}},
            "ReleaseRequest":{"type":"object","additionalProperties":false,"required":["lease_id","token","idempotency_key"],"properties":{"lease_id":{"type":"string","format":"uuid"},"token":{"type":"string","format":"uuid"},"idempotency_key":{"type":"string"}}},
            "PresenceRequest":{"type":"object","additionalProperties":false,"required":["online"],"properties":{"online":{"type":"boolean"}}},
            "Error":{"type":"object","additionalProperties":false,"required":["code","message"],"properties":{"code":{"type":"string"},"message":{"type":"string"}}},
            "Grant":{"type":"object","required":["token","preempted_lease_id","state"],"properties":{"token":{"type":"string","format":"uuid"},"preempted_lease_id":{"anyOf":[{"type":"string","format":"uuid"},{"type":"null"}]},"state":{"$ref":"#/components/schemas/TargetSnapshot"}}},
            "ReleaseResult":{"type":"object","required":["released","restored_lease_id","state"],"properties":{"released":{"type":"boolean"},"restored_lease_id":{"anyOf":[{"type":"string","format":"uuid"},{"type":"null"}]},"state":{"$ref":"#/components/schemas/TargetSnapshot"}}},
            "LeaseView":{"type":"object","required":["lease_id","source","route","priority","remaining_ms"],"properties":{"lease_id":{"type":"string","format":"uuid"},"source":{"type":"string"},"route":{"type":"string"},"priority":{"type":"integer"},"remaining_ms":{"type":"integer","format":"uint64"}}},
            "TargetSnapshot":{"type":"object","required":["target","version","desired_route","observed_route","reconciled","endpoint_online","session_eligible","advertisement_eligible","lease","suspended_leases"],"properties":{"target":{"type":"string"},"version":{"type":"integer","format":"uint64"},"desired_route":{"type":"string"},"observed_route":{"anyOf":[{"type":"string"},{"type":"null"}]},"reconciled":{"type":"boolean"},"endpoint_online":{"type":"boolean"},"session_eligible":{"type":"boolean"},"advertisement_eligible":{"type":"boolean"},"lease":{"anyOf":[{"$ref":"#/components/schemas/LeaseView"},{"type":"null"}]},"suspended_leases":{"type":"integer"}}},
            "TargetSnapshotList":{"type":"array","items":{"$ref":"#/components/schemas/TargetSnapshot"}},"Health":{"type":"object","required":["status","version"],"properties":{"status":{"type":"string"},"version":{"type":"string"}}},"OpenApiDocument":{"type":"object"}
        }}
    })
}
