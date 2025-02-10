mod helpers;

use helpers::{run_cluster, run_pike, wait_for_proc, CmdArguments, PLUGIN_DIR};
use nix::sys::signal::kill;
use nix::unistd::Pid;
use std::{
    fs::{self},
    path::Path,
    thread,
    time::{Duration, Instant},
    vec,
};

const TOTAL_INSTANCES: i32 = 4;

#[test]
fn test_cluster_setup_debug() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();
}

#[test]
fn test_cluster_setup_release() {
    let run_params = CmdArguments {
        run_args: ["--release", "--data-dir", "new_data_dir"]
            .iter()
            .map(|&s| s.into())
            .collect(),
        stop_args: ["--data-dir", "new_data_dir"]
            .iter()
            .map(|&s| s.into())
            .collect(),
        ..Default::default()
    };

    let _cluster_handle =
        run_cluster(Duration::from_secs(120), TOTAL_INSTANCES, run_params).unwrap();
}

// Using as much command line arguments in this test as we can
#[test]
fn test_cluster_daemon_and_arguments() {
    let run_params = CmdArguments {
        run_args: [
            "-d",
            "--topology",
            "../../assets/topology.toml",
            "--base-http-port",
            "8001",
            "--base-pg-port",
            "5430",
            "--target-dir",
            "tmp_target",
        ]
        .iter()
        .map(|&s| s.into())
        .collect(),
        build_args: ["--target-dir", "tmp_target"]
            .iter()
            .map(|&s| s.into())
            .collect(),
        plugin_args: vec!["--workspace".to_string()],
        ..Default::default()
    };

    let _cluster_handle =
        run_cluster(Duration::from_secs(120), TOTAL_INSTANCES, run_params).unwrap();

    // Validate each instances's PID
    for entry in fs::read_dir(Path::new(PLUGIN_DIR).join("tmp").join("cluster")).unwrap() {
        let entry = entry.unwrap();
        let pid_path = entry.path().join("pid");

        assert!(pid_path.exists());

        if let Ok(content) = fs::read_to_string(&pid_path) {
            assert!(content.trim().parse::<u32>().is_ok());
        }
    }
}

#[test]
fn test_cargo_stop() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    // Stop picodata cluster
    let mut cargo_stop_proc = run_pike(vec!["stop"], PLUGIN_DIR, &vec![]).unwrap();

    wait_for_proc(&mut cargo_stop_proc, Duration::from_secs(10));

    let start = Instant::now();
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        // Search for PID's of picodata instances and check their liveness
        let mut cluster_stopped = true;
        for entry in fs::read_dir(Path::new(PLUGIN_DIR).join("tmp").join("cluster")).unwrap() {
            let entry = entry.unwrap();
            let pid_path = entry.path().join("pid");

            if let Ok(content) = fs::read_to_string(&pid_path) {
                let pid = Pid::from_raw(content.trim().parse::<i32>().unwrap());
                // Check if proccess of picodata is still running
                if kill(pid, None).is_ok() {
                    cluster_stopped = false;
                    break;
                }
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
