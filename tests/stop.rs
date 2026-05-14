mod helpers;

use helpers::{exec_pike, run_cluster, CmdArguments, PLUGIN_DIR, PLUGIN_NAME};
use std::{
    fs::{self},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::helpers::is_instance_running;

const TOTAL_INSTANCES: i32 = 4;
const CLUSTER_STOP_TIMEOUT: Duration = Duration::from_secs(60);
const CLUSTER_START_TIMEOUT: Duration = Duration::from_secs(120);

fn assert_cluster_stopped(timeout: Duration) {
    let start = Instant::now();
    let delay = Duration::from_millis(200);

    while Instant::now().duration_since(start) < timeout {
        // Search for PID's of picodata instances and check their liveness
        let mut cluster_stopped = true;
        let cluster_dir = Path::new(PLUGIN_DIR).join("tmp").join("cluster");

        for instance_dir in fs::read_dir(cluster_dir).unwrap() {
            // Check if proccess of picodata is still running
            if is_instance_running(&instance_dir.unwrap().path()) {
                cluster_stopped = false;
                break;
            }
        }

        if cluster_stopped {
            return;
        }

        thread::sleep(delay);
    }

    panic!(
        "Timeouted while trying to stop cluster, processes with associated PID's are still running"
    );
}

#[test]
fn test_pike_stop_default() {
    let _cluster_handle = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    // Stop picodata cluster
    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert_cluster_stopped(CLUSTER_STOP_TIMEOUT);
}

#[test]
fn test_pike_stop_daemon_cluster() {
    let cmd_args = CmdArguments {
        run_args: ["--daemon"].iter().map(|&s| s.into()).collect(),
        ..Default::default()
    };
    let _cluster_handle = run_cluster(CLUSTER_START_TIMEOUT, TOTAL_INSTANCES, cmd_args)
        .expect("Failed to start cluster");

    // Stop picodata cluster
    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert_cluster_stopped(CLUSTER_STOP_TIMEOUT);
}

#[test]
fn test_pike_stop_sigterm_with_timeout() {
    let _cluster_handle = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .expect("Failed to start cluster");

    // Stop picodata cluster
    exec_pike([
        "stop",
        "--signal",
        "SIGTERM",
        "--timeout",
        "5",
        "--plugin-path",
        PLUGIN_NAME,
    ]);

    // A bit more than specified in --timeout.
    assert_cluster_stopped(Duration::from_secs(10));
}

#[test]
fn test_pike_stop_of_specific_instance() {
    let target_instance = "i2";

    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    // Stop single instance in the cluster.
    exec_pike([
        "stop",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        target_instance,
    ]);

    let data_dir = Path::new(PLUGIN_DIR).join("tmp").join("cluster");
    let instance_dir = data_dir.join(target_instance);

    // Wait while stopping instance is not killed.
    let start = Instant::now();
    let timeout = Duration::from_secs(60);

    while is_instance_running(&instance_dir) {
        thread::sleep(Duration::from_secs(1));

        assert!(
            Instant::now().duration_since(start) < timeout,
            "Timeout has reached. Instance was not stopped."
        );
    }

    // Check that all other instances were not killed.
    for entry in fs::read_dir(data_dir).unwrap() {
        let instance_dir = entry.unwrap().path();

        // Skip symlinks.
        if fs::symlink_metadata(&instance_dir).unwrap().is_symlink() {
            continue;
        }

        // Skip target instance (it's stopped).
        if instance_dir.file_name().unwrap() == target_instance {
            continue;
        }

        // All other instances should be running.
        assert!(
            is_instance_running(&instance_dir),
            "No any instance should be killed except passed in --instance-name"
        );
    }
}
