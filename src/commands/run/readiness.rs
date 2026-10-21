use crate::commands::lib::instance_info::InstanceSocketClient;
use crate::commands::{lib::timeout::Timeout, run::Params};
use crate::healthcheck::api;
use anyhow::{bail, Result};
use log::{debug, info, warn};
use std::collections::HashMap;
use std::fmt;
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

/// The calls the vshard wait loop makes to one instance on each iteration.
trait VshardInstance {
    fn name(&self) -> &str;
    fn wake_rebalancer_and_discovery(&self) -> Result<()>;
    fn wake_discovery(&self) -> Result<()>;
    fn bucket_count(&self) -> Result<u32>;
    fn vshard_replicaset_map(&self) -> Result<HashMap<String, u32>>;
}

/// Timer of the wait loop: tells the current time and waits between polls.
/// Unit tests replace it with a fake one.
trait Timer {
    fn now(&self) -> Instant;
    fn sleep(&self, duration: Duration);
}

struct SystemTimer;

impl Timer for SystemTimer {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

/// A running instance, checked through its admin socket.
struct AdminSocketInstance<'a> {
    name: &'a str,
    client: InstanceSocketClient<'a>,
}

impl VshardInstance for AdminSocketInstance<'_> {
    fn name(&self) -> &str {
        self.name
    }

    fn wake_rebalancer_and_discovery(&self) -> Result<()> {
        self.client.wake_rebalancer_and_discovery()
    }

    fn wake_discovery(&self) -> Result<()> {
        self.client.wake_discovery()
    }

    fn bucket_count(&self) -> Result<u32> {
        self.client.bucket_count()
    }

    fn vshard_replicaset_map(&self) -> Result<HashMap<String, u32>> {
        self.client.vshard_replicaset_map()
    }
}

/// Time limits for both phases of the vshard wait.
#[derive(Clone, Copy)]
struct WaitLimits {
    total_timeout: Timeout,
    idle_budget: Timeout,
}

/// The whole vshard wait: its timer, its time limits and its start time.
struct VshardWait<'a, T: Timer> {
    timer: &'a T,
    limits: WaitLimits,
    started_at: Instant,
}

impl<T: Timer> VshardWait<'_, T> {
    // Fails the phase when the total timeout or the idle budget runs out.
    // If both run out at the same time, the total timeout error is reported.
    fn check_limits<S: PartialEq>(
        &self,
        state: &PhaseState<S>,
        ready_time_of_probe: Instant,
        waiting_for: fmt::Arguments<'_>,
    ) -> Result<()> {
        let phase = state.phase;
        if ready_time_of_probe.saturating_duration_since(self.started_at)
            >= self.limits.total_timeout
        {
            bail!(
                "{phase} did not complete within total timeout {}s \
                (--wait-vshard-discovery-timeout); {waiting_for}",
                self.limits.total_timeout.as_secs()
            );
        }

        let idle = state.progress.idle_duration(ready_time_of_probe);
        if idle >= self.limits.idle_budget {
            bail!(
                "{phase} made no progress for {}s \
                (--wait-vshard-discovery-idle-timeout {}s); {waiting_for}",
                idle.as_secs(),
                self.limits.idle_budget.as_secs()
            );
        }

        Ok(())
    }
}

/// Tracks progress between iterations, and how long there has been none.
struct ProgressTracker<S> {
    last_state: Option<S>,
    last_progress_at: Instant,
}

impl<S: PartialEq> ProgressTracker<S> {
    fn new(now: Instant) -> Self {
        Self {
            last_state: None,
            last_progress_at: now,
        }
    }

    fn check_and_update(&mut self, state: S, now: Instant) -> bool {
        let changed = self
            .last_state
            .as_ref()
            .is_some_and(|last_state| *last_state != state);
        if changed {
            self.last_progress_at = now;
        }
        self.last_state = Some(state);
        changed
    }

    /// Forgets the previous state and restarts the idle countdown, because
    /// the wait moves on to the next instance. The next state is a new
    /// starting point for the comparison, not a change.
    fn reset_baseline(&mut self, now: Instant) {
        self.last_state = None;
        self.last_progress_at = now;
    }

    fn idle_duration(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_progress_at)
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Resharding,
    RouterDiscovery,
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Resharding => "vshard resharding",
            Self::RouterDiscovery => "vshard router discovery",
        })
    }
}

/// State of one wait phase. All instances of the phase share it.
struct PhaseState<S> {
    phase: Phase,
    progress: ProgressTracker<S>,
}

impl<S: PartialEq> PhaseState<S> {
    fn new(phase: Phase, now: Instant) -> Self {
        Self {
            phase,
            progress: ProgressTracker::new(now),
        }
    }

    // After each probe that finds the instance not ready, wakes up the vshard
    // fibers of the phase on every instance, so the next probe after the sleep
    // sees their work. A ready instance is accepted with no wakeups, so once
    // the cluster is ready the remaining instances cost one probe each.
    // Waking only the probed instance is not enough:
    // - Buckets reach the probed instance through the rebalancer of its tier.
    //   The rebalancer runs on one instance: the master of the replicaset with
    //   the lowest id. We do not know this instance in advance.
    // - Routers of the other instances must keep discovering at the same time.
    //   When only the probed instance was woken, router discovery was as slow as with no wakeups.
    //
    // Wakeup errors never stop the wait. Each one is logged as a warning.
    fn wake_up_instances<I: VshardInstance>(&self, instances: &[I]) {
        for instance in instances {
            let (fiber_name, result) = match self.phase {
                Phase::Resharding => (
                    "rebalancer and discovery",
                    instance.wake_rebalancer_and_discovery(),
                ),
                Phase::RouterDiscovery => ("discovery", instance.wake_discovery()),
            };
            if let Err(err) = result {
                warn!(
                    "Instance '{}': failed to wake up vshard {fiber_name}: {err:#}",
                    instance.name()
                );
            }
        }
    }
}

#[derive(Clone, Copy)]
struct BucketTarget {
    tier_replicaset_count: u32,
    tier_bucket_count: u32,
}

impl BucketTarget {
    fn is_reached_by(self, instance_bucket_count: u32) -> bool {
        (self.tier_replicaset_count * instance_bucket_count).abs_diff(self.tier_bucket_count)
            < self.tier_replicaset_count
    }

    fn expected_instance_bucket_count(self) -> u32 {
        self.tier_bucket_count
            .checked_div(self.tier_replicaset_count)
            .unwrap_or_default()
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
/// In both phases, each iteration that finds an instance not ready wakes up the vshard
/// fibers. They do not wait for the fiber timers. The wait fails if a phase makes no progress for the idle timeout, or if
/// both phases together run longer than the total timeout.
pub(super) fn wait_vshard_discovery(instances: &[PicodataInstance], params: &Params) -> Result<()> {
    let wait = VshardWait {
        timer: &SystemTimer,
        limits: WaitLimits {
            total_timeout: params.wait_vshard_discovery_timeout,
            idle_budget: params.wait_vshard_discovery_idle_timeout,
        },
        started_at: SystemTimer.now(),
    };
    info!(
        "Waiting for vshard discovery on {} instance(s) (total timeout {}s, idle timeout {}s)",
        instances.len(),
        wait.limits.total_timeout.as_secs(),
        wait.limits.idle_budget.as_secs()
    );

    let vshard_instances: Vec<AdminSocketInstance> = instances
        .iter()
        .map(|instance| AdminSocketInstance {
            name: &instance.instance_name,
            client: instance.socket_client(&params.picodata_path),
        })
        .collect();

    let bucket_count_per_replicaset = wait_for_initial_resharding(&vshard_instances, &wait)?;
    wait_for_vshard_router_init(&vshard_instances, &bucket_count_per_replicaset, &wait)?;

    info!(
        "Vshard discovery has been completed on all instances within {:.2?}",
        wait.started_at.elapsed()
    );

    Ok(())
}

// Waits for initial vshard resharding to complete on all instances.
// Returns a map of replicaset UUID → bucket count (collected from masters),
// or fails if the timeout is exceeded.
fn wait_for_initial_resharding(
    instances: &[AdminSocketInstance],
    wait: &VshardWait<SystemTimer>,
) -> Result<HashMap<String, u32>> {
    let start = wait.timer.now();
    let mut bucket_count_per_replicaset: HashMap<String, u32> = HashMap::new();
    let mut state = PhaseState::new(Phase::Resharding, start);

    info!(
        "Waiting for initial resharding (timeout {}s)",
        wait.limits.total_timeout.as_secs()
    );

    for (current, instance) in instances.iter().enumerate() {
        let target = BucketTarget {
            tier_replicaset_count: instance.client.tier_replicaset_count()?,
            tier_bucket_count: instance.client.tier_bucket_count()?,
        };

        debug!(
            "Instance '{}': awaiting resharding completion",
            instance.name
        );

        let instance_bucket_count =
            wait_for_bucket_target(instances, current, target, &mut state, wait)?;

        // If current instance is master, preserve number of buckets in the map.
        let replicaset_uuid: String = instance.client.replicaset_uuid()?;

        if instance.client.is_master_of_replicaset(&replicaset_uuid)? {
            info!("Replicaset '{replicaset_uuid}' has {instance_bucket_count} known bucket(s)");
            bucket_count_per_replicaset.insert(replicaset_uuid, instance_bucket_count);
        }
    }

    info!(
        "Initial resharding completed across all instances in {:.2?}",
        start.elapsed()
    );

    Ok(bucket_count_per_replicaset)
}

// Waits until `instances[current]` holds its share of buckets. Returns its
// bucket count at that moment. Each iteration first wakes up the rebalancer
// and discovery on all instances.
fn wait_for_bucket_target<I: VshardInstance, T: Timer>(
    instances: &[I],
    current: usize,
    target: BucketTarget,
    state: &mut PhaseState<u32>,
    wait: &VshardWait<T>,
) -> Result<u32> {
    let instance = &instances[current];
    // The next instance is compared only with its own previous values.
    state.progress.reset_baseline(wait.timer.now());

    loop {
        let instance_bucket_count = instance.bucket_count()?;
        let ready_time_of_probe = wait.timer.now();

        if target.is_reached_by(instance_bucket_count) {
            debug!(
                "Instance '{}': resharding completed (bucket_count = {})",
                instance.name(),
                instance_bucket_count
            );
            return Ok(instance_bucket_count);
        }

        if state
            .progress
            .check_and_update(instance_bucket_count, ready_time_of_probe)
        {
            debug!(
                "Instance '{}': resharding progress (bucket_count = {})",
                instance.name(),
                instance_bucket_count
            );
        }

        wait.check_limits(
            state,
            ready_time_of_probe,
            format_args!(
                "waiting for instance {} (bucket_count {}, expected {}); {}/{} instance(s) done",
                instance.name(),
                instance_bucket_count,
                target.expected_instance_bucket_count(),
                current,
                instances.len()
            ),
        )?;

        state.wake_up_instances(instances);
        wait.timer.sleep(CHECK_INTERVAL);
    }
}

/// Waits until vshard.router is initialized and synchronized on all instances,
/// or returns an error if the timeout is exceeded.
fn wait_for_vshard_router_init<I: VshardInstance, T: Timer>(
    instances: &[I],
    bucket_count_per_replicaset: &HashMap<String, u32>,
    wait: &VshardWait<T>,
) -> Result<()> {
    let start = wait.timer.now();
    let mut state: PhaseState<HashMap<String, u32>> =
        PhaseState::new(Phase::RouterDiscovery, start);

    info!("Waiting for vshard initialization");

    for (done, instance) in instances.iter().enumerate() {
        debug!(
            "Instance '{}': awaiting vshard.router initialization",
            instance.name()
        );
        // The next instance is compared only with its own previous values.
        state.progress.reset_baseline(wait.timer.now());

        loop {
            // Fetch vshard.router map from socket.
            let probe_result = instance.vshard_replicaset_map();
            let ready_time_of_probe = wait.timer.now();
            match probe_result {
                Ok(map) if map == *bucket_count_per_replicaset => {
                    debug!("Instance '{}': has synced vshard router", instance.name());
                    break;
                }
                Ok(map) => {
                    debug!(
                        "Instance '{}': vshard.router not yet synced",
                        instance.name()
                    );
                    debug!("vshard.router state: {map:?}");
                    // Only a map that was read can show progress.
                    if state.progress.check_and_update(map, ready_time_of_probe) {
                        debug!(
                            "Instance '{}': vshard.router discovery progress",
                            instance.name()
                        );
                    }
                }
                Err(err) => {
                    // Likely vshard.router not yet available, so
                    // lua returned an error.
                    debug!("Unable to get vshard replicaset map: {err:#}");
                }
            }

            wait.check_limits(
                &state,
                ready_time_of_probe,
                format_args!(
                    "waiting for instance {}; {done}/{} instance(s) done",
                    instance.name(),
                    instances.len()
                ),
            )?;

            state.wake_up_instances(instances);
            wait.timer.sleep(CHECK_INTERVAL);
        }
    }

    info!(
        "Initialization of vshard.router completed on all instances within {:.2?}",
        start.elapsed()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    struct FakeTimer {
        now: Cell<Instant>,
        sleeps: Cell<usize>,
    }

    impl FakeTimer {
        fn new() -> Self {
            Self {
                now: Cell::new(Instant::now()),
                sleeps: Cell::new(0),
            }
        }

        fn sleeps(&self) -> usize {
            self.sleeps.get()
        }
    }

    impl Timer for FakeTimer {
        fn now(&self) -> Instant {
            self.now.get()
        }

        fn sleep(&self, duration: Duration) {
            self.now.set(self.now.get() + duration);
            self.sleeps.set(self.sleeps.get() + 1);
        }
    }

    // Returns the scripted answers in order, then keeps returning the last one.
    struct FakeInstance {
        name: String,
        bucket_counts: RefCell<VecDeque<u32>>,
        maps: RefCell<VecDeque<Result<HashMap<String, u32>, String>>>,
        resharding_wakes: Cell<usize>,
        discovery_wakes: Cell<usize>,
    }

    impl FakeInstance {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                bucket_counts: RefCell::default(),
                maps: RefCell::default(),
                resharding_wakes: Cell::new(0),
                discovery_wakes: Cell::new(0),
            }
        }

        fn with_bucket_counts(name: &str, bucket_counts: &[u32]) -> Self {
            let instance = Self::new(name);
            instance.bucket_counts.borrow_mut().extend(bucket_counts);
            instance
        }

        fn with_maps(name: &str, maps: Vec<Result<HashMap<String, u32>, String>>) -> Self {
            let instance = Self::new(name);
            instance.maps.borrow_mut().extend(maps);
            instance
        }
    }

    fn take_next_answer<T: Clone>(answers: &RefCell<VecDeque<T>>) -> T {
        let mut answers = answers.borrow_mut();
        if answers.len() > 1 {
            answers.pop_front().expect("checked above")
        } else {
            answers
                .front()
                .cloned()
                .expect("fake instance has no scripted answers")
        }
    }

    impl VshardInstance for FakeInstance {
        fn name(&self) -> &str {
            &self.name
        }

        fn wake_rebalancer_and_discovery(&self) -> Result<()> {
            self.resharding_wakes.set(self.resharding_wakes.get() + 1);
            Ok(())
        }

        fn wake_discovery(&self) -> Result<()> {
            self.discovery_wakes.set(self.discovery_wakes.get() + 1);
            Ok(())
        }

        fn bucket_count(&self) -> Result<u32> {
            Ok(take_next_answer(&self.bucket_counts))
        }

        fn vshard_replicaset_map(&self) -> Result<HashMap<String, u32>> {
            take_next_answer(&self.maps).map_err(anyhow::Error::msg)
        }
    }

    const TARGET: BucketTarget = BucketTarget {
        tier_replicaset_count: 2,
        tier_bucket_count: 3000,
    };

    const LIMITS: WaitLimits = WaitLimits {
        total_timeout: Timeout::from_secs(600),
        idle_budget: Timeout::from_secs(60),
    };

    fn fake_wait(timer: &FakeTimer, limits: WaitLimits) -> VshardWait<'_, FakeTimer> {
        VshardWait {
            timer,
            limits,
            started_at: timer.now(),
        }
    }

    fn router_map(counts: &[(&str, u32)]) -> HashMap<String, u32> {
        counts
            .iter()
            .map(|&(uuid, count)| (uuid.to_string(), count))
            .collect()
    }

    #[test]
    fn check_and_update_restarts_idle_countdown_on_change() {
        let start = Instant::now();
        let mut tracker = ProgressTracker::new(start);
        assert!(!tracker.check_and_update(1, start));
        let later = start + Duration::from_secs(5);
        assert!(tracker.check_and_update(2, later));
        assert_eq!(
            tracker.idle_duration(later + Duration::from_secs(1)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn check_and_update_keeps_idle_countdown_without_change() {
        let start = Instant::now();
        let mut tracker = ProgressTracker::new(start);
        assert!(!tracker.check_and_update(1, start));
        let later = start + Duration::from_secs(5);
        assert!(!tracker.check_and_update(1, later));
        assert_eq!(tracker.idle_duration(later), Duration::from_secs(5));
    }

    #[test]
    fn reset_baseline_restarts_idle_countdown() {
        let start = Instant::now();
        let mut tracker = ProgressTracker::new(start);
        assert!(!tracker.check_and_update(1, start));
        let later = start + Duration::from_secs(5);
        // The wait moves on to the next instance. The idle countdown starts
        // again, and the first value of that instance is not a change.
        tracker.reset_baseline(later);
        assert_eq!(tracker.idle_duration(later), Duration::ZERO);
        let first_probe = later + Duration::from_secs(1);
        assert!(!tracker.check_and_update(7, first_probe));
        assert_eq!(tracker.idle_duration(first_probe), Duration::from_secs(1));
    }

    #[test]
    fn bucket_target_wait_returns_on_iteration_after_ready() {
        let timer = FakeTimer::new();
        let instances = [FakeInstance::with_bucket_counts("i1", &[3000, 2000, 1500])];
        let mut state = PhaseState::new(Phase::Resharding, timer.now());
        let count = wait_for_bucket_target(
            &instances,
            0,
            TARGET,
            &mut state,
            &fake_wait(&timer, LIMITS),
        )
        .unwrap();
        assert_eq!(count, 1500);
        assert_eq!(timer.sleeps(), 2);

        let timer = FakeTimer::new();
        let instances = [FakeInstance::with_bucket_counts("i1", &[1500])];
        let mut state = PhaseState::new(Phase::Resharding, timer.now());
        let count = wait_for_bucket_target(
            &instances,
            0,
            TARGET,
            &mut state,
            &fake_wait(&timer, LIMITS),
        )
        .unwrap();
        assert_eq!(count, 1500);
        assert_eq!(timer.sleeps(), 0);
    }

    #[test]
    fn bucket_target_wait_fails_at_once_with_zero_idle_budget() {
        let limits = WaitLimits {
            idle_budget: Timeout::from_secs(0),
            ..LIMITS
        };

        let timer = FakeTimer::new();
        let instances = [FakeInstance::with_bucket_counts("i1", &[3000, 1500])];
        let mut state = PhaseState::new(Phase::Resharding, timer.now());
        let err = wait_for_bucket_target(
            &instances,
            0,
            TARGET,
            &mut state,
            &fake_wait(&timer, limits),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("made no progress"), "{err:#}");
        assert_eq!(timer.sleeps(), 0);

        let timer = FakeTimer::new();
        let instances = [FakeInstance::with_bucket_counts("i1", &[1500])];
        let mut state = PhaseState::new(Phase::Resharding, timer.now());
        let count = wait_for_bucket_target(
            &instances,
            0,
            TARGET,
            &mut state,
            &fake_wait(&timer, limits),
        )
        .unwrap();
        assert_eq!(count, 1500);
    }

    #[test]
    fn bucket_target_wait_wakes_instances_only_while_not_ready() {
        let timer = FakeTimer::new();
        let instances = [
            FakeInstance::with_bucket_counts("i1", &[3000, 1500]),
            FakeInstance::with_bucket_counts("i2", &[1500]),
        ];
        let mut state = PhaseState::new(Phase::Resharding, timer.now());
        let wait = fake_wait(&timer, LIMITS);
        wait_for_bucket_target(&instances, 0, TARGET, &mut state, &wait).unwrap();
        // One probe of i1 found it not ready, so every instance was woken once.
        for instance in &instances {
            assert_eq!(instance.resharding_wakes.get(), 1, "{}", instance.name);
            assert_eq!(instance.discovery_wakes.get(), 0, "{}", instance.name);
        }

        // i2 is ready on the first probe, so nobody is woken.
        wait_for_bucket_target(&instances, 1, TARGET, &mut state, &wait).unwrap();
        for instance in &instances {
            assert_eq!(instance.resharding_wakes.get(), 1, "{}", instance.name);
        }
    }

    #[test]
    fn router_init_returns_on_iteration_after_ready() {
        let timer = FakeTimer::new();
        let expected = router_map(&[("rs1", 1500), ("rs2", 1500)]);
        let instances = [
            FakeInstance::with_maps(
                "i1",
                vec![
                    Err("not ready".to_string()),
                    Ok(router_map(&[("rs1", 3000)])),
                    Ok(expected.clone()),
                ],
            ),
            FakeInstance::with_maps("i2", vec![Ok(expected.clone())]),
        ];
        wait_for_vshard_router_init(&instances, &expected, &fake_wait(&timer, LIMITS)).unwrap();
        assert_eq!(timer.sleeps(), 2);
    }

    #[test]
    fn router_init_restarts_idle_budget() {
        let limits = WaitLimits {
            idle_budget: Timeout::from_secs(10),
            ..LIMITS
        };
        let timer = FakeTimer::new();
        let wait_started_at = timer.now();
        // Pretend the resharding phase used 9 of the 10 idle seconds.
        timer.sleep(Duration::from_secs(9));

        let expected = router_map(&[("rs1", 1500), ("rs2", 1500)]);
        let stale = router_map(&[("rs1", 3000)]);
        let mut answers = vec![Ok(stale); 4];
        answers.push(Ok(expected.clone()));
        let instances = [FakeInstance::with_maps("i1", answers)];
        let wait = VshardWait {
            timer: &timer,
            limits,
            started_at: wait_started_at,
        };
        wait_for_vshard_router_init(&instances, &expected, &wait).unwrap();
    }

    #[test]
    fn router_init_probe_error_is_not_progress() {
        let limits = WaitLimits {
            total_timeout: Timeout::from_secs(5),
            idle_budget: Timeout::from_secs(2),
        };
        let timer = FakeTimer::new();
        let expected = router_map(&[("rs1", 1500), ("rs2", 1500)]);
        let stale = router_map(&[("rs1", 3000)]);
        // Keeps alternating for longer than the total timeout.
        let answers = (0..40)
            .map(|i| {
                if i % 2 == 0 {
                    Ok(stale.clone())
                } else {
                    Err("not ready".to_string())
                }
            })
            .collect();
        let instances = [FakeInstance::with_maps("i1", answers)];
        let err = wait_for_vshard_router_init(&instances, &expected, &fake_wait(&timer, limits))
            .unwrap_err();
        assert!(format!("{err:#}").contains("made no progress"), "{err:#}");
    }

    #[test]
    fn router_init_wakes_only_discovery_while_not_ready() {
        let timer = FakeTimer::new();
        let expected = router_map(&[("rs1", 1500), ("rs2", 1500)]);
        let instances = [
            FakeInstance::with_maps(
                "i1",
                vec![Ok(router_map(&[("rs1", 3000)])), Ok(expected.clone())],
            ),
            FakeInstance::with_maps("i2", vec![Ok(expected.clone())]),
        ];
        wait_for_vshard_router_init(&instances, &expected, &fake_wait(&timer, LIMITS)).unwrap();
        // Only the first probe of i1 found an instance not ready.
        for instance in &instances {
            assert_eq!(instance.resharding_wakes.get(), 0, "{}", instance.name);
            assert_eq!(instance.discovery_wakes.get(), 1, "{}", instance.name);
        }
    }

    #[test]
    fn router_init_counts_idle_time_from_switch_to_next_instance() {
        let limits = WaitLimits {
            idle_budget: Timeout::from_secs(10),
            ..LIMITS
        };
        let timer = FakeTimer::new();
        let expected = router_map(&[("rs1", 1500), ("rs2", 1500)]);
        // i1 keeps the same map for 9 of the 10 idle seconds, then syncs.
        let mut i1_answers = vec![Ok(router_map(&[("rs1", 3000)])); 19];
        i1_answers.push(Ok(expected.clone()));
        // The router of i2 cannot be read for the first probes.
        let i2_answers = vec![
            Err("not ready".to_string()),
            Err("not ready".to_string()),
            Err("not ready".to_string()),
            Ok(expected.clone()),
        ];
        let instances = [
            FakeInstance::with_maps("i1", i1_answers),
            FakeInstance::with_maps("i2", i2_answers),
        ];
        wait_for_vshard_router_init(&instances, &expected, &fake_wait(&timer, limits)).unwrap();
    }
}
