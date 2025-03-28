mod helpers;

use helpers::{exec_pike, run_cluster, CmdArguments, PLUGIN_DIR, PLUGIN_NAME};
use nix::sys::signal::kill;
use nix::unistd::Pid;
use std::{
    fs::{self},
    path::Path,
    thread,
    time::{Duration, Instant},
};

const TOTAL_INSTANCES: i32 = 4;

#[test]
fn test_cargo_stop() {
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
