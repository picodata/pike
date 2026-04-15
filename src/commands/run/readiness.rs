use crate::commands::lib::run_query_in_picodata_admin;
use crate::commands::run::Params;
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

        println!("WTF: ready count: {ready_count}");

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

/// Polls vshard router state via admin socket on each instance until
/// `bucket_count: {active_buckets}` appears in `vshard.router` output,
/// indicating that vshard discovery has settled.
pub(super) fn wait_vshard_discovery(instances: &[PicodataInstance], params: &Params) -> Result<()> {
    let timeout = Duration::from_secs(params.wait_vshard_discovery_timeout);
    info!(
        "Waiting for vshard discovery on {} instance(s) (timeout {}s)",
        instances.len(),
        timeout.as_secs()
    );

    for instance in instances {
        let socket_path = instance.data_dir().join("admin.sock");
        let start = Instant::now();

        let active_buckets = fetch_instance_buckets(instances, instance, &start, timeout)?;
        let needle = format!("bucket_count: {active_buckets}");

        loop {
            if start.elapsed() >= timeout {
                bail!(
                    "vshard discovery timed out: '{needle}' not found in vshard.router output \
                     on instance {} within {}s",
                    instance.http_port(),
                    timeout.as_secs()
                );
            }

            println!("WTF: search for: {needle}");

            let output = run_query_in_picodata_admin(
                &params.picodata_path,
                &socket_path,
                "\\lua\nvshard.router",
            );

            println!("WTF: vshard.router output: {output:?}");

            match output {
                Ok(stdout) if stdout.contains(&needle) => break,
                Ok(_) => {}
                Err(e) => {
                    debug!(
                        "vshard.router query failed on instance {}: {e}",
                        instance.http_port()
                    );
                }
            }

            thread::sleep(CHECK_INTERVAL);
        }
    }

    info!("vshard discovery has been completed on all instances");
    Ok(())
}

/// Polls `/api/v1/health/status` until **all** instances report `Healthy`,
/// then returns `buckets.active` for the requested instance.
fn fetch_instance_buckets(
    instances: &[PicodataInstance],
    instance: &PicodataInstance,
    start: &Instant,
    timeout: Duration,
) -> Result<usize> {
    loop {
        if start.elapsed() >= timeout {
            bail!(
                "vshard discovery timed out: not all instances became healthy within {}s",
                timeout.as_secs()
            );
        }

        let statuses: Vec<_> = instances.iter().map(api::get_health_status).collect();

        {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            for (inst, status) in instances.iter().zip(statuses.iter()) {
                match status {
                    Ok(s) => println!(
                        "{ts} - WTF - instance port={} status={:?} buckets.active={} buckets.total={}",
                        inst.http_port(),
                        s.status,
                        s.buckets.active,
                        s.buckets.total,
                    ),
                    Err(e) => println!(
                        "{ts} - WTF - instance port={} status=ERROR err={e}",
                        inst.http_port(),
                    ),
                }
            }
        }

        let all_healthy = statuses
            .iter()
            .all(|s| matches!(s, Ok(s) if s.status == api::HealthStatusLevel::Healthy));

        if all_healthy {
            let active = statuses
                .into_iter()
                .zip(instances.iter())
                .find(|(_, i)| i.http_port() == instance.http_port())
                .expect("instance must be in the list")
                .0?
                .buckets
                .active;
            return Ok(active);
        }

        debug!("not all instances are healthy yet, retrying...");

        thread::sleep(CHECK_INTERVAL);
    }
}
