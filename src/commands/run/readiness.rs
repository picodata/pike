use crate::commands::run::Params;
use crate::healthcheck::api;
use anyhow::{bail, Result};
use log::{debug, info};
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use super::PicodataInstance;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Polls startup and readiness probes on each instance until all return 200,
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

/// Waits for vshard discovery to complete across all instances.
///
/// The routine ensures the cluster reaches a consistent and fully initialized state in two phases:
///
/// 1. Initial resharding:
///    Waits until buckets are evenly distributed across instances. For each replicaset,
///    collects the number of buckets from master instances.
///
/// 2. Vshard initialization:
///    Waits until `vshard.router` on every instance reflects the actual cluster state by
///    verifying that bucket counts per replicaset match those observed on storage.
///
pub(super) fn wait_vshard_discovery(instances: &[PicodataInstance], params: &Params) -> Result<()> {
    let timeout = Duration::from_secs(params.wait_vshard_discovery_timeout);
    info!(
        "Waiting for vshard discovery on {} instance(s) (timeout {}s)",
        instances.len(),
        timeout.as_secs()
    );

    let start = Instant::now();
    let picodata_path = &params.picodata_path;
    let bucket_count_per_replicaset =
        wait_for_initial_resharding(instances, picodata_path, timeout)?;

    let Some(time_left) = timeout.checked_sub(start.elapsed()) else {
        bail!("no time left for vshard initialization");
    };

    wait_for_vshard_router_init(
        instances,
        picodata_path,
        &bucket_count_per_replicaset,
        time_left,
    )?;

    info!(
        "Vshard discovery has been completed on all instances within {:.2?}",
        start.elapsed()
    );

    Ok(())
}

// Waits for initial vshard resharding to complete on all instances.
// Returns a map of replicaset UUID → bucket count (collected from masters),
// or fails if the timeout is exceeded.
fn wait_for_initial_resharding(
    instances: &[PicodataInstance],
    picodata_path: &PathBuf,
    timeout: Duration,
) -> Result<HashMap<String, u32>> {
    let start = Instant::now();
    let mut bucket_count_per_replicaset: HashMap<String, u32> = HashMap::new();

    info!(
        "Waiting for initial resharding (timeout {}s)",
        timeout.as_secs()
    );

    for instance in instances {
        let instance_socket = instance.socket_client(picodata_path);
        let tier_replicaset_count: u32 = instance_socket.tier_replicaset_count()?;
        let tier_bucket_count: u32 = instance_socket.tier_bucket_count()?;

        // Wait for the current instance to complete resharding.

        debug!(
            "Instance '{}': awaiting resharding completion",
            instance.instance_name
        );

        let instance_bucket_count = loop {
            let instance_bucket_count: u32 = instance_socket.bucket_count()?;

            if (tier_replicaset_count * instance_bucket_count).abs_diff(tier_bucket_count)
                < tier_replicaset_count
            {
                debug!(
                    "Instance '{}': resharding completed (bucket_count = {})",
                    instance.instance_name, instance_bucket_count
                );
                break instance_bucket_count;
            }

            if start.elapsed() >= timeout {
                bail!(
                    "Resharding timed out on instance '{}' within {}s",
                    instance.instance_name,
                    timeout.as_secs()
                );
            }

            thread::sleep(CHECK_INTERVAL);
        };

        // If current instance is master, preserve number of buckets in the map.
        let replicaset_uuid: String = instance_socket.replicaset_uuid()?;

        if instance_socket.is_master_of_replicaset(&replicaset_uuid)? {
            info!("Replicaset '{replicaset_uuid}' has {instance_bucket_count} known bucket(s)",);
            bucket_count_per_replicaset.insert(replicaset_uuid, instance_bucket_count);
        }
    }

    info!(
        "Initial resharding completed across all instances in {:.2?}",
        start.elapsed()
    );

    Ok(bucket_count_per_replicaset)
}

/// Waits until vshard.router is initialized and synchronized on all instances,
/// or returns an error if the timeout is exceeded.
fn wait_for_vshard_router_init(
    instances: &[PicodataInstance],
    picodata_path: &PathBuf,
    bucket_count_per_replicaset: &HashMap<String, u32>,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();

    info!(
        "Waiting for vshard initialization (timeout {}s)",
        timeout.as_secs()
    );

    for instance in instances {
        let instance_socket = instance.socket_client(picodata_path);

        debug!(
            "Instance '{}': awaiting vshard.router initialization",
            instance.instance_name
        );

        loop {
            // Fetch vshard.router map from socket.
            match instance_socket.vshard_replicaset_map() {
                Ok(map) if map == *bucket_count_per_replicaset => {
                    debug!(
                        "Instance '{}': has synced vshard router",
                        instance.instance_name
                    );
                    break;
                }
                Ok(map) => {
                    debug!(
                        "Instance '{}': vshard.router not yet synced",
                        instance.instance_name
                    );
                    debug!("vshard.router state: {map:?}");
                }
                Err(err) => {
                    // Likely vshard.router not yet available, so
                    // lua returned an error.
                    debug!("Unable to get vshard replicaset map: {err:}");
                }
            }

            if start.elapsed() >= timeout {
                bail!(
                    "Initialization of vshard.router timed out within {}s",
                    timeout.as_secs()
                );
            }

            thread::sleep(CHECK_INTERVAL);
        }
    }

    info!(
        "Initialization of vshard.router completed on all instances within {:.2?}",
        start.elapsed()
    );

    Ok(())
}
