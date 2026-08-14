use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use household_audio_router::{
    AcquireRequest, AdapterPolicy, Authority, ReporterPolicy, RouteDriver, TargetConfig,
    api::{ApiState, AuthConfig, openapi_document, router},
};
use http_body_util::BodyExt;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;
struct Fake(Mutex<String>);
#[async_trait]
impl RouteDriver for Fake {
    async fn set_route(&self, _: &str, r: &str) -> Result<(), String> {
        *self.0.lock().await = r.into();
        Ok(())
    }
    async fn observe_route(&self, _: &str) -> Result<String, String> {
        Ok(self.0.lock().await.clone())
    }
    async fn probe(&self) -> Result<(), String> {
        Ok(())
    }
}
fn state_and_app() -> (ApiState, axum::Router) {
    let authority = Authority::new(
        vec![TargetConfig {
            name: "room".into(),
            idle_route: "idle".into(),
            driver_target: "g".into(),
        }],
        Arc::new(Fake(Mutex::new("idle".into()))),
    )
    .unwrap();
    let policy = AdapterPolicy {
        source: "airplay".into(),
        priority: 10,
        targets: HashSet::from(["room".into()]),
        routes: HashSet::from(["music".into()]),
        min_ttl_ms: 1,
        max_ttl_ms: 1_000,
        resume_preempted: false,
    };
    let state = ApiState::new(
        authority,
        AuthConfig {
            adapters: HashMap::from([("adapter-secret".into(), policy)]),
            reporters: HashMap::from([(
                "reporter-secret".into(),
                ReporterPolicy {
                    reporter: "ma-presence".into(),
                    targets: HashSet::from(["room".into()]),
                },
            )]),
            readers: HashSet::from(["read-secret".into()]),
            admins: HashSet::from(["admin-secret".into()]),
        },
    );
    let app = router(state.clone());
    (state, app)
}
fn app() -> axum::Router {
    state_and_app().1
}
async fn body_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test(start_paused = true)]
async fn fail_closed_auth_separates_adapter_read_and_admin() {
    let app = app();
    let req = Request::post("/api/v1/targets/room/lease")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(response).await["code"], "unauthorized");
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/state")
                .header("authorization", "Bearer adapter-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            Request::get("/metrics")
                .header("authorization", "Bearer read-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .oneshot(
            Request::get("/metrics")
                .header("authorization", "Bearer admin-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
#[tokio::test(start_paused = true)]
async fn unknown_get_is_404_and_json_errors_are_stable() {
    let app = app();
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/targets/missing")
                .header("authorization", "Bearer read-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(response).await["code"], "unknown_target");
    let malformed = app
        .clone()
        .oneshot(
            Request::post("/api/v1/targets/room/lease")
                .header("authorization", "Bearer adapter-secret")
                .header("content-type", "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(body_json(malformed).await["code"], "invalid_json");
    let injected=app.oneshot(Request::post("/api/v1/targets/room/lease").header("authorization","Bearer adapter-secret").header("content-type","application/json").body(Body::from(r#"{"lease_id":"00000000-0000-0000-0000-000000000001","idempotency_key":"x","route":"music","ttl_ms":10,"priority":999}"#)).unwrap()).await.unwrap();
    assert_eq!(injected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(injected).await["code"], "invalid_json");
}
#[tokio::test(start_paused = true)]
async fn authenticated_presence_then_acquire_uses_server_identity() {
    let app = app();
    let denied = app
        .clone()
        .oneshot(
            Request::put("/api/v1/targets/room/presence")
                .header("authorization", "Bearer adapter-secret")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"online":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let online = app
        .clone()
        .oneshot(
            Request::put("/api/v1/targets/room/presence")
                .header("authorization", "Bearer reporter-secret")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"online":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(online.status(), StatusCode::OK);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    let req = AcquireRequest {
        lease_id: Uuid::new_v4(),
        idempotency_key: "a".into(),
        route: "music".into(),
        ttl_ms: 100,
    };
    let response = app
        .oneshot(
            Request::post("/api/v1/targets/room/lease")
                .header("authorization", "Bearer adapter-secret")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        body_json(response).await["state"]["lease"]["source"],
        "airplay"
    );
}
#[test]
fn openapi_is_versioned_and_documents_secured_operations() {
    let doc = openapi_document();
    assert_eq!(doc["openapi"], "3.1.0");
    for path in [
        "/healthz",
        "/readyz",
        "/metrics",
        "/openapi.json",
        "/api/v1/targets/{target}",
        "/api/v1/targets/{target}/lease",
        "/api/v1/targets/{target}/presence",
        "/api/v1/state",
        "/api/v1/events",
    ] {
        assert!(doc["paths"].get(path).is_some(), "missing {path}");
    }
    assert_eq!(
        doc["components"]["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );
}

#[tokio::test]
async fn sse_reports_lag_gap_and_shutdown_cancels_stream() {
    let (state, app) = state_and_app();
    let response = app
        .oneshot(
            Request::get("/api/v1/events")
                .header("authorization", "Bearer read-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let reporter = state.auth.reporters.get("reporter-secret").unwrap().clone();
    for i in 0..300 {
        state
            .authority
            .presence("room", &reporter, i % 2 == 0)
            .await
            .unwrap();
    }
    let mut body = response.into_body();
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .expect("lag event is prompt")
        .expect("stream has a frame")
        .unwrap()
        .into_data()
        .expect("SSE frame is data");
    let text = String::from_utf8(first.to_vec()).unwrap();
    assert!(text.contains("id: gap-"));
    assert!(text.contains("event: gap"));
    assert!(text.contains("refetch"));
    state.shutdown();
    tokio::time::timeout(std::time::Duration::from_secs(1), body.collect())
        .await
        .expect("SSE shutdown is bounded")
        .unwrap();
}

#[tokio::test]
async fn fallback_errors_are_stable_json() {
    let app = app();
    let missing = app
        .clone()
        .oneshot(Request::get("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(missing).await["code"], "not_found");
    let method = app
        .oneshot(Request::patch("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body_json(method).await["code"], "method_not_allowed");
}

#[tokio::test(start_paused = true)]
async fn readyz_fails_immediately_after_presence_and_acquire_commits() {
    let (state, app) = state_and_app();
    state.authority.tick().await;
    let ok = app
        .clone()
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let reporter = state.auth.reporters.get("reporter-secret").unwrap();
    state
        .authority
        .presence("room", reporter, true)
        .await
        .unwrap();
    let down = app
        .clone()
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    state.authority.tick().await;
    let adapter = state.auth.adapters.get("adapter-secret").unwrap();
    state
        .authority
        .acquire(
            "room",
            adapter,
            AcquireRequest {
                lease_id: Uuid::new_v4(),
                idempotency_key: "ready".into(),
                route: "music".into(),
                ttl_ms: 100,
            },
        )
        .await
        .unwrap();
    let down = app
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(down.status(), StatusCode::SERVICE_UNAVAILABLE);
}
#[test]
fn openapi_validates_and_payload_fixtures_match() {
    let doc = openapi_document();
    assert_eq!(doc["openapi"], "3.1.0");
    assert!(!doc.to_string().contains("nullable"));
    for schema in ["RenewRequest", "ReleaseRequest"] {
        let s = &doc["components"]["schemas"][schema];
        assert_eq!(s["additionalProperties"], false);
        assert!(s["properties"].is_object());
        assert!(s["required"].as_array().unwrap().len() >= 3);
    }
    for path in [
        "/api/v1/targets/{target}/lease",
        "/api/v1/targets/{target}/presence",
    ] {
        for operation in doc["paths"][path].as_object().unwrap().values() {
            for status in ["404", "409", "502", "503"] {
                let error = &operation["responses"][status]["content"]["application/json"]["schema"]
                    ["$ref"];
                assert_eq!(error, "#/components/schemas/Error");
            }
        }
    }
    let renew = serde_json::json!({"lease_id":Uuid::nil(),"token":Uuid::nil(),"idempotency_key":"fixture","ttl_ms":100});
    serde_json::from_value::<household_audio_router::RenewRequest>(renew).unwrap();
    let release =
        serde_json::json!({"lease_id":Uuid::nil(),"token":Uuid::nil(),"idempotency_key":"fixture"});
    serde_json::from_value::<household_audio_router::ReleaseRequest>(release).unwrap();
    let snapshot = serde_json::json!({"target":"room","version":1,"desired_route":"music","observed_route":"idle","reconciled":false,"endpoint_online":true,"session_eligible":true,"advertisement_eligible":true,"lease":null,"suspended_leases":0});
    let grant =
        serde_json::json!({"token":Uuid::nil(),"preempted_lease_id":null,"state":snapshot.clone()});
    serde_json::from_value::<household_audio_router::Grant>(grant).unwrap();
    let released = serde_json::json!({"released":true,"restored_lease_id":null,"state":snapshot});
    serde_json::from_value::<household_audio_router::ReleaseResult>(released.clone()).unwrap();
    let validate_response = |path: &str,
                             method: &str,
                             status: &str,
                             instance: &serde_json::Value| {
        let response_ref = doc["paths"][path][method]["responses"][status]["content"]["application/json"]["schema"]["$ref"].as_str().unwrap();
        let root = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","components":doc["components"].clone(),"$ref":response_ref});
        let validator = jsonschema::draft202012::options()
            .build(&root)
            .expect("valid JSON Schema 2020-12 response schema");
        validator
            .validate(instance)
            .expect("null-bearing response fixture validates");
    };
    let grant = serde_json::json!({"token":Uuid::nil(),"preempted_lease_id":null,"state":serde_json::json!({"target":"room","version":1,"desired_route":"idle","observed_route":null,"reconciled":false,"endpoint_online":false,"session_eligible":false,"advertisement_eligible":false,"lease":null,"suspended_leases":0})});
    validate_response("/api/v1/targets/{target}/lease", "post", "201", &grant);
    validate_response("/api/v1/targets/{target}", "get", "200", &grant["state"]);
    validate_response("/api/v1/targets/{target}/lease", "delete", "200", &released);
    assert_eq!(
        doc["paths"]["/api/v1/targets/{target}/lease"]["post"]["responses"]["201"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/Grant"
    );
}

#[tokio::test(start_paused = true)]
async fn unchanged_presence_heartbeat_is_noop_and_keeps_readyz() {
    let (state, app) = state_and_app();
    let reporter = state.auth.reporters.get("reporter-secret").unwrap();
    state
        .authority
        .presence("room", reporter, true)
        .await
        .unwrap();
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    state.authority.tick().await;
    assert!(state.authority.is_ready());
    let before = state.authority.snapshot("room").await.unwrap();
    let mut events = state.authority.subscribe();
    let heartbeat = state
        .authority
        .presence("room", reporter, true)
        .await
        .unwrap();
    assert_eq!(heartbeat.version, before.version);
    assert!(state.authority.is_ready());
    assert!(events.try_recv().is_err());
    let response = app
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
