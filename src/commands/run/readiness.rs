use crate::healthcheck::api;
use anyhow::{bail, Result};
use log::{debug, info};
use std::thread;
use std::time::{Duration, Instant};

use super::PicodataInstance;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Polls `GET /api/v1/health/ready` on each instance until all return 200,
/// or until the timeout is exceeded.
pub(super) fn wait_instances_ready(instances: &[PicodataInstance]) -> Result<()> {
    if instances.is_empty() {
        return Ok(());
    }

    info!(
        "Waiting for {} instance(s) to become ready (timeout {}s)",
        instances.len(),
        HEALTH_CHECK_TIMEOUT.as_secs()
    );

    let start = Instant::now();

    loop {
        if start.elapsed() >= HEALTH_CHECK_TIMEOUT {
            bail!(
                "cluster setup timed out: not all instances became ready within {}s",
                HEALTH_CHECK_TIMEOUT.as_secs()
            );
        }

        let ready_count = instances
            .iter()
            .map(api::is_instance_ready)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|&ready| ready)
            .count();

        if ready_count == instances.len() {
            info!("All {} instance(s) are ready", instances.len());
            return Ok(());
        }

        debug!(
            "{}/{} instance(s) ready, retrying...",
            ready_count,
            instances.len()
        );
        thread::sleep(CHECK_INTERVAL);
    }
}
