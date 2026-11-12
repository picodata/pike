mod helpers;

use helpers::{
    exec_pike, init_plugin, run_cluster, wait_instance_offline, CmdArguments, CLUSTER_DIR,
    PLUGIN_DIR, PLUGIN_NAME, TESTS_DIR,
};
use pike::cluster::{Tier, Topology};
use pike::status::{self, InstancesParamsBuilder, ProcessState};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const TOTAL_INSTANCES: i32 = 4;
const CLUSTER_START_TIMEOUT: Duration = Duration::from_secs(240);
const LEADER_MOVE_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

fn daemon_args() -> CmdArguments {
    CmdArguments {
        run_args: vec![
            "--daemon".to_string(),
            "--wait-instance-online-timeout".to_string(),
            "120".to_string(),
        ],
        ..Default::default()
    }
}

/// Runs `pike status instances` and returns (success, stdout, stderr).
// Pike leaves a supervisor process holding the pipes until the parent is
// reaped, so wait for the exit status first and only then read the output.
fn status_instances(args: &[&str]) -> (bool, String, String) {
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let mut pike_child = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .arg("status")
        .arg("instances")
        .args(args)
        .current_dir(TESTS_DIR)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to execute pike");

    let exit_status = pike_child.wait().unwrap();

    let mut stdout = String::new();
    pike_child
        .stdout
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let mut stderr = String::new();
    pike_child
        .stderr
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();

    (exit_status.success(), stdout, stderr)
}

/// Status as parsed JSON; asserts that the run succeeded.
fn status_json() -> Value {
    let (ok, stdout, stderr) =
        status_instances(&["--plugin-path", PLUGIN_NAME, "--format", "json"]);
    assert!(ok, "pike status instances failed: {stderr}");
    serde_json::from_str(&stdout).expect("stdout is not valid JSON")
}

fn instances_of(status: &Value) -> &Vec<Value> {
    status["instances"].as_array().unwrap()
}

/// Resolves the Picodata name of an instance directory (`i<N>`) by its
/// symlink.
fn instance_name(dir_name: &str) -> String {
    fs::read_dir(CLUSTER_DIR)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| fs::read_link(path).is_ok_and(|target| target == Path::new(dir_name)))
        .unwrap_or_else(|| panic!("no picodata name of '{dir_name}'"))
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// Rewrites `replicasets = 2` of the test plugin topology.
fn set_topology_replicasets(replicasets: u8) {
    let path = Path::new(PLUGIN_DIR).join("topology.toml");
    let content = fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("replicasets = 2"),
        "unexpected topology.toml: {content}"
    );
    fs::write(
        &path,
        content.replace("replicasets = 2", &format!("replicasets = {replicasets}")),
    )
    .unwrap();
}

// The library API works without a cluster: the expected instances are
// considered as not started.
#[test]
fn test_status_api_without_cluster() {
    let params = InstancesParamsBuilder::default()
        .topology(default_topology())
        .plugin_path(PathBuf::from("/nonexistent/pike-status-api-smoke"))
        .build()
        .unwrap();

    let collected = status::collect(&params).unwrap();

    assert_eq!(collected.instances.len(), 2);
    assert_eq!(collected.unexpected, Vec::<String>::new());
    assert!(collected
        .instances
        .iter()
        .all(|instance| instance.process == ProcessState::NotStarted));

    status::instances(&params).unwrap();
}

fn default_topology() -> Topology {
    Topology {
        tiers: BTreeMap::from([(
            "default".to_string(),
            Tier {
                replicasets: 1,
                replication_factor: 2,
            },
        )]),
        ..Default::default()
    }
}

#[test]
fn test_status_cluster_lifetime() {
    // start cluster
    let _cluster = run_cluster(CLUSTER_START_TIMEOUT, TOTAL_INSTANCES, daemon_args()).unwrap();

    let status = status_json();
    let instances = instances_of(&status);

    assert_eq!(instances.len(), TOTAL_INSTANCES as usize);
    assert_eq!(status["unexpected"], json!([]));

    let leaders = instances
        .iter()
        .filter(|instance| instance["raft_leader"].as_bool() == Some(true))
        .count();
    assert_eq!(leaders, 1, "exactly one raft leader expected: {status}");

    // after start all instances have running/online states
    let mut replicasets: BTreeMap<&str, usize> = BTreeMap::new();
    for instance in instances {
        assert_eq!(instance["process"], "running");
        assert_eq!(instance["current_state"], "Online");
        assert_eq!(instance["tier"], "default");
        assert!(instance["name"].is_string());
        *replicasets
            .entry(instance["replicaset"].as_str().unwrap())
            .or_insert(0) += 1;
    }
    // two replicasets with two instances each
    assert_eq!(replicasets.len(), 2);
    assert!(replicasets.values().all(|count| *count == 2));

    // stop only one instance
    exec_pike([
        "stop",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        "i2",
    ]);
    wait_instance_offline(&instance_name("i2"));

    // the remaining instances are running so we can connect to picodata and
    // fetch all information we need to fill in all table fields
    let status = status_json();
    for instance in instances_of(&status) {
        if instance["dir"].as_str() == Some("i2") {
            assert_eq!(instance["process"], "stopped");
            assert_eq!(instance["current_state"], "Offline");
        } else {
            assert_eq!(instance["process"], "running");
            assert_eq!(instance["current_state"], "Online");
        }
    }

    // stop the whole cluster
    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    let (ok, stdout, stderr) =
        status_instances(&["--plugin-path", PLUGIN_NAME, "--format", "json"]);
    assert!(ok, "status instances failed: {stderr}");
    assert!(
        !stderr.contains("failed to fetch cluster data"),
        "no warning expected for a stopped cluster: {stderr}"
    );

    // after the cluster is stopped all instances have stopped/null
    let status: Value = serde_json::from_str(&stdout).unwrap();
    for instance in instances_of(&status) {
        assert_eq!(instance["process"], "stopped");
        assert_eq!(instance["tier"], "default");
        // name symlinks survive the stop
        assert!(instance["name"].is_string());
        assert_eq!(instance["replicaset"], json!(null));
        assert_eq!(instance["current_state"], json!(null));
        assert_eq!(instance["raft_leader"], json!(null));
    }
}

#[test]
fn test_status_after_expel() {
    // start cluster
    let _cluster = run_cluster(CLUSTER_START_TIMEOUT, TOTAL_INSTANCES, daemon_args()).unwrap();

    let status = status_json();
    let instances = instances_of(&status);

    assert_eq!(instances.len(), TOTAL_INSTANCES as usize);
    assert_eq!(status["unexpected"], json!([]));

    let leaders = instances
        .iter()
        .filter(|instance| instance["raft_leader"].as_bool() == Some(true))
        .count();
    assert_eq!(leaders, 1, "exactly one raft leader expected: {status}");

    // expel a non-leader instance
    let ins_to_expel = instances
        .iter()
        .find(|instance| instance["raft_leader"].as_bool() == Some(false))
        .and_then(|instance| instance["dir"].as_str())
        .expect("a running cluster has a non-leader instance")
        .to_string();

    // expel waits for the Expelled state itself, and with --force also
    // for the process exit, so the cluster view is settled afterwards
    exec_pike([
        "expel",
        "--plugin-path",
        PLUGIN_NAME,
        "--force",
        &ins_to_expel,
    ]);

    // the expelled instance keeps its row in _pico_instance
    let status = status_json();
    for instance in instances_of(&status) {
        if instance["dir"].as_str() == Some(ins_to_expel.as_str()) {
            assert_eq!(instance["process"], "stopped");
            assert_eq!(instance["current_state"], "Expelled");
            assert_eq!(instance["tier"], "default");
            assert!(instance["name"].is_string());
            assert!(instance["replicaset"].is_string());
        } else {
            assert_eq!(instance["process"], "running");
            assert_eq!(instance["current_state"], "Online");
        }
    }
    // the instance stays described in topology.toml
    assert_eq!(status["unexpected"], json!([]));
}

// After the leader is stopped, the leader flag moves to another instance
// once the elections complete.
#[test]
fn test_status_after_raft_leader_election() {
    fn leader_dir(status: &Value) -> Option<&str> {
        instances_of(status)
            .iter()
            .find(|instance| instance["raft_leader"].as_bool() == Some(true))
            .and_then(|instance| instance["dir"].as_str())
    }

    let _cluster = run_cluster(CLUSTER_START_TIMEOUT, TOTAL_INSTANCES, daemon_args()).unwrap();

    let old_leader = leader_dir(&status_json())
        .expect("a running cluster has to have a leader")
        .to_string();

    exec_pike([
        "stop",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        &old_leader,
    ]);

    let start = Instant::now();
    loop {
        // Right after the stop the cluster may still have no leader for some
        // time, so wait until the leader flag settles on another instance
        if leader_dir(&status_json()).is_some_and(|leader| leader != old_leader) {
            break;
        }
        assert!(
            start.elapsed() < LEADER_MOVE_TIMEOUT,
            "raft leader did not move after stopping {old_leader} within the {}s",
            LEADER_MOVE_TIMEOUT.as_secs()
        );
        thread::sleep(POLL_INTERVAL);
    }
}

// Instances of an increased topology are considered as not started.
#[test]
fn test_status_replicasets_increased() {
    let _cluster = run_cluster(CLUSTER_START_TIMEOUT, TOTAL_INSTANCES, daemon_args()).unwrap();
    let status = status_json();
    assert_eq!(instances_of(&status).len(), TOTAL_INSTANCES as usize);

    // increase the number of replicasets in topology.toml after the cluster started
    set_topology_replicasets(3);

    let status = status_json();
    let instances = instances_of(&status);
    assert_eq!(instances.len(), 6);
    assert_eq!(status["unexpected"], json!([]));

    for instance in instances {
        match instance["dir"].as_str().unwrap() {
            // new replicasets are added to the output list and considered not started
            "i5" | "i6" => {
                assert_eq!(instance["process"], "not_started");
                assert_eq!(instance["current_state"], json!(null));
            }
            _ => {
                assert_eq!(instance["process"], "running");
                assert_eq!(instance["current_state"], "Online");
            }
        }
    }

    // additionally the text summary reports the not started instances
    let (ok, stdout, _) = status_instances(&["--plugin-path", PLUGIN_NAME]);
    assert!(ok);
    assert!(stdout.contains("2 not started"));
}

// Directories of a decreased topology are considered as unexpected while the
// instances themselves keep running.
#[test]
fn test_status_replicasets_decreased() {
    let _cluster = run_cluster(CLUSTER_START_TIMEOUT, TOTAL_INSTANCES, daemon_args()).unwrap();
    let status = status_json();
    assert_eq!(instances_of(&status).len(), TOTAL_INSTANCES as usize);

    // decrease the number of replicasets in topology.toml after the cluster started
    set_topology_replicasets(1);

    let status = status_json();
    // so instances that vanished from topology.toml become unexpected
    assert_eq!(status["unexpected"], json!(["i3", "i4"]));

    for instance in instances_of(&status) {
        assert_eq!(instance["process"], "running");
        assert_eq!(instance["current_state"], "Online");
    }

    // additionally the text summary reports the unexpected instances
    let (ok, stdout, _) = status_instances(&["--plugin-path", PLUGIN_NAME]);
    assert!(ok);
    assert!(stdout.contains("2 not described in topology.toml: i3, i4"));
}

// If the cluster directory is missing, the cluster is considered not started
#[test]
fn test_status_no_cluster_dir() {
    // a fresh plugin: the topology.toml must exist, but no cluster ever ran
    init_plugin(PLUGIN_NAME);

    let nonexistent_dir = "/nonexistent/pike-status-folder";
    assert!(!Path::new(nonexistent_dir).exists());

    let (ok, stdout, stderr) = status_instances(&[
        "--plugin-path",
        PLUGIN_NAME,
        "--format",
        "json",
        "--data-dir",
        nonexistent_dir,
    ]);
    assert!(ok, "status instances failed: {stderr}");
    assert!(
        !stderr.contains("failed to fetch cluster data"),
        "no warning expected without a cluster: {stderr}"
    );

    let status: Value = serde_json::from_str(&stdout).unwrap();
    let instances = instances_of(&status);
    assert_eq!(instances.len(), TOTAL_INSTANCES as usize);
    assert!(instances
        .iter()
        .all(|instance| instance["process"] == "not_started"));
    assert_eq!(status["unexpected"], json!([]));
}
