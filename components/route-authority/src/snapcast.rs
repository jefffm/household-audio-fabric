//! Snapcast 0.35 HTTP JSON-RPC route driver.
use crate::RouteDriver;
use async_trait::async_trait;
use reqwest::{Client, header::CONNECTION};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct SnapcastDriver {
    endpoint: String,
    client: Client,
    next_id: AtomicU64,
}
impl SnapcastDriver {
    #[must_use]
    pub fn new(endpoint: String) -> Self {
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(1))
            .timeout(std::time::Duration::from_secs(2))
            .pool_max_idle_per_host(0)
            .build()
            .expect("fixed reqwest client configuration is valid");
        Self {
            endpoint,
            client,
            next_id: AtomicU64::new(1),
        }
    }
    async fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .client
            .post(&self.endpoint)
            .header(CONNECTION, "close")
            .json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("Snapcast HTTP {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| e.to_string())?;
        if let Some(error) = body.get("error") {
            return Err(format!("Snapcast RPC error: {error}"));
        }
        body.get("result")
            .cloned()
            .ok_or_else(|| "Snapcast response has no result".into())
    }
    async fn status(&self) -> Result<Value, String> {
        self.rpc("Server.GetStatus", json!({})).await
    }
}
#[async_trait]
impl RouteDriver for SnapcastDriver {
    async fn set_route(&self, group: &str, route: &str) -> Result<(), String> {
        let status = self.status().await?;
        let current = find_group_stream(&status, group)?;
        if current == route {
            return Ok(());
        }
        self.rpc("Group.SetStream", json!({"id":group,"stream_id":route}))
            .await
            .map(|_| ())
    }
    async fn observe_route(&self, group: &str) -> Result<String, String> {
        find_group_stream(&self.status().await?, group)
    }
    async fn probe(&self) -> Result<(), String> {
        self.status().await.map(|_| ())
    }
}
fn find_group_stream(status: &Value, group: &str) -> Result<String, String> {
    status
        .pointer("/server/groups")
        .and_then(Value::as_array)
        .and_then(|groups| {
            groups
                .iter()
                .find(|g| g.get("id").and_then(Value::as_str) == Some(group))
        })
        .and_then(|g| g.get("stream_id").or_else(|| g.get("streamId")))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("Snapcast group {group} missing from Server.GetStatus"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_snapcast_035_status() {
        let status = json!({"server":{"groups":[{"id":"kitchen","stream_id":"radio"}]}});
        assert_eq!(find_group_stream(&status, "kitchen").unwrap(), "radio");
        assert!(find_group_stream(&status, "missing").is_err());
    }
}
