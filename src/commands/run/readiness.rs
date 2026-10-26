use crate::commands::{
    lib::{instance_info::InstanceSocketClient, timeout::Timeout},
    run::Params,
};
use crate::healthcheck::api;
use anyhow::{bail, ensure, Context, Result};
use log::{debug, info};
use std::collections::HashMap;
use std::ops::RangeInclusive;
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
            .collect::<Vec<_>>()
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
/// On every polling iteration the vshard rebalancer or router discovery
/// is woken up on each instance, so that waiting doesn't depend on
/// internal vshard timers. Wakeup failures are ignored, and failed reads
/// of the `vshard.router` state are retried until a timeout.
///
/// Both phases share one total timeout (`wait_vshard_discovery_timeout`). Each phase
/// also fails if nothing changes for longer than the idle timeout
/// (`wait_vshard_idle_timeout`).
pub(super) fn wait_vshard_discovery(instances: &[PicodataInstance], params: &Params) -> Result<()> {
    let start = Instant::now();
    let policy = PollPolicy::new(
        params.wait_vshard_discovery_timeout,
        params.wait_vshard_idle_timeout,
        CHECK_INTERVAL,
    );
    info!(
        "Waiting for vshard discovery on {} instance(s) (timeout {}s, idle timeout {}s)",
        instances.len(),
        policy.total_timeout.as_secs(),
        policy.idle_timeout.as_secs()
    );

    let picodata_path = &params.picodata_path;
    let bucket_count_per_replicaset =
        wait_for_initial_resharding(instances, picodata_path, &policy)?;
    wait_for_vshard_router_init(
        instances,
        picodata_path,
        &bucket_count_per_replicaset,
        &policy,
    )?;

    info!(
        "Vshard discovery has been completed on all instances within {:.2?}",
        start.elapsed()
    );

    Ok(())
}

/// Waits for initial vshard resharding to complete on all instances.
/// Returns a map of replicaset UUID → bucket count (collected from masters),
/// or fails if the total or idle timeout is exceeded.
fn wait_for_initial_resharding(
    instances: &[PicodataInstance],
    picodata_path: &PathBuf,
    policy: &PollPolicy,
) -> Result<HashMap<String, u32>> {
    let start = Instant::now();
    info!("Waiting for initial resharding");

    let expected_bucket_counts = collect_expected_bucket_counts(instances, picodata_path)
        .context("unable to collect expected bucket counts")?;
    let mut poller = Poller::new(policy);

    let bucket_count_per_replicaset = loop {
        wakeup_rebalancer(instances, picodata_path);

        let bucket_count_per_instance =
            read_all(instances, picodata_path, InstanceSocketClient::bucket_count)?;
        if is_resharding_complete(
            instances,
            &expected_bucket_counts,
            &bucket_count_per_instance,
        ) {
            // The master role is checked after resharding: it may change while waiting.
            break collect_master_bucket_counts(
                instances,
                picodata_path,
                &bucket_count_per_instance,
            )?;
        }

        poller
            .wait_next(&bucket_count_per_instance)
            .context("waiting for initial resharding")?;
    };

    info!(
        "Initial resharding completed across all instances in {:.2?}",
        start.elapsed()
    );

    Ok(bucket_count_per_replicaset)
}

fn collect_expected_bucket_counts(
    instances: &[PicodataInstance],
    picodata_path: &PathBuf,
) -> Result<HashMap<String, RangeInclusive<u32>>> {
    let mut result = HashMap::new();
    for instance in instances {
        if result.contains_key(&instance.tier) {
            continue;
        }
        let client = instance.socket_client(picodata_path);
        let replicaset_count = client.tier_replicaset_count()?;
        let bucket_count = client.tier_bucket_count()?;
        ensure!(
            replicaset_count > 0,
            "tier '{}' has no replicasets",
            instance.tier
        );
        let expected = bucket_count / replicaset_count..=bucket_count.div_ceil(replicaset_count);
        result.insert(instance.tier.clone(), expected);
    }
    Ok(result)
}

fn is_resharding_complete(
    instances: &[PicodataInstance],
    expected_bucket_counts: &HashMap<String, RangeInclusive<u32>>,
    bucket_count_per_instance: &HashMap<String, u32>,
) -> bool {
    instances.iter().all(|instance| {
        let expected = expected_bucket_counts.get(&instance.tier).unwrap();
        let bucket_count = bucket_count_per_instance
            .get(&instance.instance_name)
            .unwrap();
        expected.contains(bucket_count)
    })
}

fn collect_master_bucket_counts(
    instances: &[PicodataInstance],
    picodata_path: &PathBuf,
    bucket_count_per_instance: &HashMap<String, u32>,
) -> Result<HashMap<String, u32>> {
    let mut result = HashMap::new();
    for instance in instances {
        let name = &instance.instance_name;
        let client = instance.socket_client(picodata_path);
        let replicaset_uuid = client
            .replicaset_uuid()
            .with_context(|| format!("instance '{name}'"))?;
        let is_master = client
            .is_master_of_replicaset(&replicaset_uuid)
            .with_context(|| format!("instance '{name}'"))?;
        if is_master {
            let bucket_count = *bucket_count_per_instance.get(name).unwrap();
            info!("Replicaset '{replicaset_uuid}' has {bucket_count} known bucket(s)");
            result.insert(replicaset_uuid, bucket_count);
        }
    }
    Ok(result)
}

/// Waits until vshard.router is initialized and synchronized on all instances,
/// or returns an error if the total or idle timeout is exceeded.
fn wait_for_vshard_router_init(
    instances: &[PicodataInstance],
    picodata_path: &PathBuf,
    bucket_count_per_replicaset: &HashMap<String, u32>,
    policy: &PollPolicy,
) -> Result<()> {
    let start = Instant::now();
    info!("Waiting for vshard initialization");

    let mut router_view_per_instance = HashMap::new();
    let mut poller = Poller::new(policy);
    loop {
        wakeup_discovery(instances, picodata_path);

        match read_all(
            instances,
            picodata_path,
            InstanceSocketClient::vshard_replicaset_map,
        ) {
            Ok(view) => {
                router_view_per_instance = view;
                if router_view_per_instance
                    .values()
                    .all(|view| view == bucket_count_per_replicaset)
                {
                    break;
                }
            }
            Err(err) => debug!("unable to read vshard replicaset map: {err:#}"),
        }

        poller
            .wait_next(&router_view_per_instance)
            .context("waiting for vshard initialization")?;
    }

    info!(
        "Initialization of vshard.router completed on all instances within {:.2?}",
        start.elapsed()
    );

    Ok(())
}

fn wakeup_rebalancer(instances: &[PicodataInstance], picodata_path: &PathBuf) {
    for instance in instances {
        if let Err(err) = instance.socket_client(picodata_path).rebalancer_wakeup() {
            debug!(
                "instance '{}': rebalancer wakeup failed: {err:#}",
                instance.instance_name
            );
        }
    }
}

fn wakeup_discovery(instances: &[PicodataInstance], picodata_path: &PathBuf) {
    for instance in instances {
        if let Err(err) = instance
            .socket_client(picodata_path)
            .router_discovery_wakeup()
        {
            debug!(
                "instance '{}': router discovery wakeup failed: {err:#}",
                instance.instance_name
            );
        }
    }
}

/// Fails on the first failed read.
fn read_all<'a, V>(
    instances: &[PicodataInstance],
    picodata_path: &'a PathBuf,
    read: impl Fn(&InstanceSocketClient<'a>) -> Result<V>,
) -> Result<HashMap<String, V>> {
    instances
        .iter()
        .map(|instance| {
            let name = &instance.instance_name;
            let value = read(&instance.socket_client(picodata_path))
                .with_context(|| format!("instance '{name}'"))?;
            Ok((name.clone(), value))
        })
        .collect()
}

/// Limits for polling vshard readiness, shared by both phases.
struct PollPolicy {
    deadline: Instant,
    /// Duration `deadline` was computed from, for messages.
    total_timeout: Duration,
    /// A phase fails if its observed state doesn't change for this long.
    idle_timeout: Duration,
    interval: Duration,
}

impl PollPolicy {
    fn new(total_timeout: Timeout, idle_timeout: Timeout, interval: Duration) -> Self {
        let total_timeout = total_timeout.into();
        Self {
            deadline: Instant::now() + total_timeout,
            total_timeout,
            idle_timeout: idle_timeout.into(),
            interval,
        }
    }
}

struct Poller<'a, V> {
    policy: &'a PollPolicy,
    last_observed: Option<V>,
    last_progress: Instant,
    last_tick: Instant,
}

impl<'a, V: Clone + PartialEq> Poller<'a, V> {
    fn new(policy: &'a PollPolicy) -> Self {
        let now = Instant::now();
        Self {
            policy,
            last_observed: None,
            last_progress: now,
            last_tick: now,
        }
    }

    /// Records `observed` and sleeps until the next iteration.
    ///
    /// Fails instead of sleeping if the deadline has passed or `observed`
    /// hasn't changed for the idle timeout.
    fn wait_next(&mut self, observed: &V) -> Result<()> {
        if self.last_observed.as_ref() != Some(observed) {
            self.last_observed = Some(observed.clone());
            self.last_progress = Instant::now();
        }

        if Instant::now() >= self.policy.deadline {
            bail!(
                "timed out within {}s (total timeout)",
                self.policy.total_timeout.as_secs(),
            );
        }

        if self.last_progress.elapsed() >= self.policy.idle_timeout {
            bail!(
                "no progress within {}s (idle timeout)",
                self.policy.idle_timeout.as_secs(),
            );
        }

        let until_next_tick = self
            .policy
            .interval
            .saturating_sub(self.last_tick.elapsed());
        let until_deadline = self
            .policy
            .deadline
            .saturating_duration_since(Instant::now());
        thread::sleep(until_next_tick.min(until_deadline));
        self.last_tick = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_millis(10);
    const IDLE_TIMEOUT: Duration = Duration::from_millis(200);
    const NO_TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

    fn policy(total_timeout: Duration, idle_timeout: Duration) -> PollPolicy {
        PollPolicy::new(total_timeout.into(), idle_timeout.into(), INTERVAL)
    }

    fn poll_for(poller: &mut Poller<u32>, state: u32, duration: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < duration {
            poller.wait_next(&state)?;
        }
        Ok(())
    }

    #[test]
    fn unchanged_state_fails_with_idle_timeout() {
        let policy = policy(NO_TOTAL_TIMEOUT, IDLE_TIMEOUT);
        let mut poller = Poller::new(&policy);
        let start = Instant::now();

        let err = poll_for(&mut poller, 1, NO_TOTAL_TIMEOUT).unwrap_err();

        assert!(err.to_string().contains("idle timeout"), "{err:#}");
        assert!(start.elapsed() >= IDLE_TIMEOUT);
    }

    #[test]
    fn changed_state_resets_idle_timeout() {
        let policy = policy(NO_TOTAL_TIMEOUT, IDLE_TIMEOUT);
        let mut poller = Poller::new(&policy);

        // Each stall is shorter than the idle timeout, but together they are longer.
        let stall = IDLE_TIMEOUT * 3 / 4;
        poll_for(&mut poller, 1, stall).unwrap();
        poll_for(&mut poller, 2, stall).unwrap();
        poll_for(&mut poller, 3, stall).unwrap();
    }

    #[test]
    fn continuous_progress_fails_with_total_timeout() {
        let total_timeout = Duration::from_millis(100);
        let start = Instant::now();
        let policy = policy(total_timeout, NO_TOTAL_TIMEOUT);
        let mut poller = Poller::new(&policy);

        let err = (0..)
            .find_map(|state: u32| poller.wait_next(&state).err())
            .unwrap();

        assert!(err.to_string().contains("total timeout"), "{err:#}");
        assert!(start.elapsed() >= total_timeout);
    }
}
