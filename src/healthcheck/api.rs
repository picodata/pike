#![allow(dead_code)]

use crate::commands::run::PicodataInstance;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const SESSION_ENDPOINT: &str = "api/v1/session";
const READINESS_ENDPOINT: &str = "api/v1/health/ready";
const STARTUP_ENDPOINT: &str = "api/v1/health/startup";
const HEALTH_STATUS_ENDPOINT: &str = "api/v1/health/status";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
enum Probe {
    Readiness,
    Startup,
}

impl Probe {
    fn path(&self) -> &'static str {
        match self {
            Probe::Startup => STARTUP_ENDPOINT,
            Probe::Readiness => READINESS_ENDPOINT,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatusLevel {
    Healthy,
    Degraded,
    #[default]
    Unhealthy,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaftStatus {
    pub state: String,
    pub term: u64,
    pub leader_id: u64,
    pub leader_name: String,
    pub applied_index: u64,
    pub commited_index: u64,
    pub compacted_index: u64,
    pub persisted_index: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketStatus {
    pub active: usize,
    pub total: usize,
    pub pinned: usize,
    pub sending: usize,
    pub receiving: usize,
    pub garbage: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterStatus {
    pub uuid: String,
    pub version: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HealthStatus {
    pub status: HealthStatusLevel,
    pub reasons: Vec<String>,
    pub timestamp: u64,
    pub uptime_seconds: u64,
    pub name: String,
    pub uuid: String,
    pub version: String,
    pub raft_id: u64,
    pub tier: String,
    pub replicaset: String,
    pub current_state: String,
    pub target_state: String,
    pub target_state_reason: Option<String>,
    pub target_state_change_time: Option<String>,
    pub limbo_owner: u64,
    pub raft: RaftStatus,
    pub buckets: BucketStatus,
    pub cluster: ClusterStatus,
}

#[derive(Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct SessionToken {
    pub auth: String,
    pub refresh: String,
}

fn build_client() -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into()
}

/// Assembles URL of instance probe.
fn build_probe_url(i: &PicodataInstance, probe: &Probe) -> String {
    format!("http://127.0.0.1:{}/{}", i.http_port(), probe.path())
}

/// Performs the specified probe on the instance via HTTP.
/// Returns `true` if the response status is successful.
fn check_instance_probe(
    http_client: &ureq::Agent,
    instance: &PicodataInstance,
    probe: &Probe,
) -> Result<bool> {
    let url = build_probe_url(instance, probe);
    let response = http_client.get(url).call()?;

    Ok(response.status().is_success())
}

/// Authenticates against `/api/v1/session` and returns JWT tokens.
pub fn get_session_token(http_port: u16, username: &str, password: &str) -> Result<SessionToken> {
    let url = format!("http://127.0.0.1:{http_port}/{SESSION_ENDPOINT}");
    let tokens = build_client()
        .post(&url)
        .send_json(&LoginRequest { username, password })?
        .body_mut()
        .read_json::<SessionToken>()?;
    Ok(tokens)
}

/// Fetches `/api/v1/health/status`.
///
/// When `with_web_auth` is `true`, logs in via `/api/v1/session` first and
/// attaches the resulting Bearer token. When `false`, the request is sent
/// without authentication (assumes JWT auth is disabled).
pub fn get_health_status(instance: &PicodataInstance) -> Result<HealthStatus> {
    let url = format!(
        "http://127.0.0.1:{}/{HEALTH_STATUS_ENDPOINT}",
        instance.http_port()
    );
    let mut resp = build_client().get(&url).call()?;
    if !resp.status().is_success() {
        bail!(
            "health status request failed with status: {}",
            resp.status()
        );
    }
    Ok(resp.body_mut().read_json::<HealthStatus>()?)
}

/// Instance is ready, when it's started and ready to accept incoming traffic.
///
/// This routine polls "/startup" endpoint and on success polls "/ready".
///
/// Returns "true" if both returned `HTTP_OK`.
///
pub fn is_instance_ready(instance: &PicodataInstance) -> Result<bool> {
    let http_client = build_client();
    let check_probe = |p| check_instance_probe(&http_client, instance, p);
    let is_ready = check_probe(&Probe::Startup)? && check_probe(&Probe::Readiness)?;

    Ok(is_ready)
}
