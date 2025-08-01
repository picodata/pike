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

#[test]
fn test_pike_stop() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    // Stop picodata cluster
    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    let start = Instant::now();
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        // Search for PID's of picodata instances and check their liveness
        let mut cluster_stopped = true;
        for instance_dir in fs::read_dir(Path::new(PLUGIN_DIR).join("tmp").join("cluster")).unwrap()
        {
            // Check if proccess of picodata is still running
            if is_instance_running(&instance_dir.unwrap().path()) {
                cluster_stopped = false;
                break;
            }
        }

        if cluster_stopped {
            return;
        }

        thread::sleep(Duration::from_secs(1));
    }

    panic!(
        "Timeouted while trying to stop cluster, processes with associated PID's are still running"
    );
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
