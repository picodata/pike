mod helpers;

use helpers::{
    prepare_vshard_plugin_dir, read_instance_name, read_raw_vshard_state, vshard_run_params,
    vshard_topology, VshardClusterGuard,
};
use pike::cluster::run;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

// Must match CHECK_INTERVAL in src/commands/run/readiness.rs.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

// Must match the wait loop in src/commands/run/readiness.rs. Once the cluster
// stops changing, every probe finds its instance ready. So after the last change
// pike makes at most: the wakeup round after the last probe that found an
// instance not ready (1 call per instance), one CHECK_INTERVAL sleep, the rest
// of the resharding phase (7 calls per ready instance) and the router phase
// (1 call per ready instance). That is 9 `picodata admin` calls per instance,
// and an observer round makes 1 call per instance.
const PIKE_TAIL_ROUNDS: u32 = 9;

// Each test here starts a full picodata cluster. The tests things (readiness
// lag, rebalance speed vs idle budget) break when clusters fight for CPU.
// cargo runs the tests of one binary in parallel, so we run them one at a time.
// PoisonError::into_inner keeps one failed test from blocking the others.
static CLUSTER_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn stalled_rebalance_fails_with_idle_error() {
    // With the rebalancer frozen, the wait can succeed only when vshard
    // bootstrap already spread the buckets evenly.
    // Then there is nothing to stall, so we start the cluster again.
    const ATTEMPTS: usize = 3;

    let _lock = CLUSTER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let stall_script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/assets/vshard_stall.lua");

    for _ in 0..ATTEMPTS {
        let plugin_path = prepare_vshard_plugin_dir("vshard-stall", None);
        let _cluster = VshardClusterGuard::new(&plugin_path);

        let mut topology = vshard_topology();
        topology.enviroment.insert(
            "PICODATA_SCRIPT".to_string(),
            stall_script.to_str().unwrap().to_string(),
        );
        let params = vshard_run_params(&plugin_path, 200)
            .topology(topology)
            .wait_vshard_discovery_idle_timeout(Duration::from_secs(10))
            .wait_vshard_discovery_timeout(Duration::from_secs(120))
            .build()
            .unwrap();

        let started_at = Instant::now();
        let Err(err) = run(params) else {
            continue;
        };
        let elapsed = started_at.elapsed();

        assert!(format!("{err:#}").contains("made no progress"), "{err:#}");
        assert!(elapsed >= Duration::from_secs(10), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(120), "{elapsed:?}");
        return;
    }

    panic!(
        "vshard bootstrap spread the buckets evenly {ATTEMPTS} times in a row, \
        so the stall could not be reproduced"
    );
}

#[test]
fn slow_rebalance_does_not_hit_idle_budget() {
    let _lock = CLUSTER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    // Enough buckets that the wait lasts much longer than the idle budget even
    // when vshard bootstrap already spreads them and only discovery is left.
    let plugin_path = prepare_vshard_plugin_dir("vshard-slow", Some(240_000));
    let _cluster = VshardClusterGuard::new(&plugin_path);

    // Long enough to cover pauses of vshard itself that our wakeups do not remove
    let idle_timeout = Duration::from_secs(30);
    let params = vshard_run_params(&plugin_path, 100)
        .wait_vshard_discovery_idle_timeout(idle_timeout)
        .wait_vshard_discovery_timeout(Duration::from_secs(600))
        .build()
        .unwrap();

    let started_at = Instant::now();
    run(params).expect("progressing rebalancing must not fail the vshard wait");
    let elapsed = started_at.elapsed();

    // Instances start in a few seconds. So if the run takes this long, the vshard
    // wait itself lasted longer than the idle budget.
    assert!(elapsed > 2 * idle_timeout, "{elapsed:?}");
}

#[test]
fn progressing_rebalance_fails_on_total_timeout() {
    let _lock = CLUSTER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let plugin_path = prepare_vshard_plugin_dir("vshard-total-timeout", Some(60000));
    let _cluster = VshardClusterGuard::new(&plugin_path);

    let params = vshard_run_params(&plugin_path, 300)
        .wait_vshard_discovery_timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let err = run(params).expect_err("the total timeout must fail the vshard wait");
    assert!(format!("{err:#}").contains("total timeout"), "{err:#}");
}

struct Observation {
    last_change_at: Option<Instant>,
    max_round: Duration,
}

// Reads the raw vshard state of every instance in a loop, with no pauses,
// until `stop` is set. Records the time of the last state change and the
// longest round. Once the cluster is ready, its state stops changing.
fn observe_vshard_state(admin_sockets: &[PathBuf], stop: &AtomicBool) -> Observation {
    let mut observation = Observation {
        last_change_at: None,
        max_round: Duration::ZERO,
    };
    let mut last_state: Option<Vec<Option<String>>> = None;

    while !stop.load(Ordering::Relaxed) {
        let round_start = Instant::now();
        let state: Vec<Option<String>> = admin_sockets
            .iter()
            .map(|socket| read_raw_vshard_state(socket))
            .collect();
        let round = round_start.elapsed();

        // Count rounds only after every instance answers.
        if state.iter().all(Option::is_some) {
            observation.max_round = observation.max_round.max(round);
            if last_state.as_ref() != Some(&state) {
                observation.last_change_at = Some(Instant::now());
                last_state = Some(state);
            }
        } else {
            thread::sleep(Duration::from_millis(100));
        }
    }

    observation
}

#[test]
fn fast_cluster_wait_finishes_right_after_ready() {
    let _lock = CLUSTER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let plugin_path = prepare_vshard_plugin_dir("vshard-fast", None);
    let _cluster = VshardClusterGuard::new(&plugin_path);

    let admin_sockets: Vec<PathBuf> = (1..=4)
        .map(|id| plugin_path.join(format!("tmp/cluster/i{id}/admin.sock")))
        .collect();
    let stop = Arc::new(AtomicBool::new(false));
    let observer = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || observe_vshard_state(&admin_sockets, &stop))
    };

    let params = vshard_run_params(&plugin_path, 400).build().unwrap();
    let result = run(params);
    let pike_done_at = Instant::now();
    stop.store(true, Ordering::Relaxed);
    let observation = observer.join().unwrap();
    result.expect("vshard wait must succeed on a fast cluster");

    let last_change_at = observation
        .last_change_at
        .expect("observer never saw every instance answer");
    let max_round = observation.max_round;
    assert!(
        max_round <= CHECK_INTERVAL,
        "environment too slow to check the readiness lag: observer round {max_round:?} > {CHECK_INTERVAL:?}",
    );
    let lag = pike_done_at.saturating_duration_since(last_change_at);
    assert!(
        lag <= CHECK_INTERVAL + PIKE_TAIL_ROUNDS * max_round,
        "pike finished waiting {lag:?} after the cluster stopped changing \
        (allowed {CHECK_INTERVAL:?} + {PIKE_TAIL_ROUNDS} x observer round {max_round:?})",
    );
}

#[test]
fn wake_error_on_one_instance_is_logged() {
    let _lock = CLUSTER_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let plugin_path = prepare_vshard_plugin_dir("vshard-wake-fail", None);
    let _cluster = VshardClusterGuard::new(&plugin_path);

    let wrapper = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/assets/picodata_wake_fail.sh");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-pike"))
        .args([
            "pike",
            "run",
            "--daemon",
            "--no-build",
            "--disable-install-plugins",
        ])
        .arg("--plugin-path")
        .arg(&plugin_path)
        .args([
            "--base-bin-port",
            "3500",
            "--base-http-port",
            "8500",
            "--base-pg-port",
            "5932",
        ])
        .arg("--picodata-path")
        .arg(&wrapper)
        .env("PIKE_TEST_FAIL_SOCKET", "/i2/admin.sock")
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");

    // picodata names instances in the order they join, so ask each instance
    // for its name. If an instance took long to become Online, pike keeps its
    // directory name (`iN`) instead and uses it in the logs, so check both.
    for instance_dir in ["i1", "i2", "i3", "i4"] {
        let admin_socket = plugin_path.join(format!("tmp/cluster/{instance_dir}/admin.sock"));
        let name = read_instance_name(&admin_socket)
            .unwrap_or_else(|| panic!("instance {instance_dir} must answer"));
        let warned = [name.as_str(), instance_dir]
            .iter()
            .any(|name| stderr.contains(&format!("Instance '{name}': failed to wake up vshard")));
        assert_eq!(
            warned,
            instance_dir == "i2",
            "{instance_dir} ({name}): {stderr}"
        );
    }
}
