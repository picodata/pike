mod helpers;

use helpers::{
    exec_pike, init_plugin, run_cluster, CmdArguments, CLUSTER_DIR, PLUGIN_DIR, PLUGIN_NAME,
    TESTS_DIR,
};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};
use toml_edit::DocumentMut;

const TOTAL_INSTANCES: i32 = 4;
const CLUSTER_START_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a stopped instance or a new raft leader may take to show up in the status.
const STATUS_STATE_CHANGE_TIMEOUT: Duration = Duration::from_secs(60);
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const CLUSTER_DATA_UNAVAILABLE_WARNING: &str = "cluster data is unavailable";

fn make_pike_status_instances_command(work_dir: &Path, extra_args: &[&str]) -> Command {
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut command = Command::new(format!("{root_dir}/target/debug/cargo-pike"));
    command
        .args(["pike", "status", "instances"])
        .args(extra_args)
        .current_dir(work_dir);
    command
}

fn run_pike_status_instances(work_dir: &Path, extra_args: &[&str]) -> Output {
    make_pike_status_instances_command(work_dir, extra_args)
        .output()
        .expect("failed to execute pike status instances")
}

fn describe_exit_status_stdout_and_stderr(output: &Output) -> String {
    format!(
        "exit status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_exit_code(output: &Output, expected_exit_code: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected_exit_code),
        "{}",
        describe_exit_status_stdout_and_stderr(output)
    );
}

fn assert_stdout_contains(output: &Output, expected_text: &str) {
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(expected_text),
        "no {expected_text:?} in stdout:\n{}",
        describe_exit_status_stdout_and_stderr(output)
    );
}

fn assert_success_without_warning_and_parse_json_report(output: &Output) -> Value {
    assert_exit_code(output, 0);
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(CLUSTER_DATA_UNAVAILABLE_WARNING),
        "unexpected warning: {}",
        describe_exit_status_stdout_and_stderr(output)
    );
    parse_json_report(output)
}

fn parse_json_report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "status output is not JSON: {err}\n{}",
            describe_exit_status_stdout_and_stderr(output)
        )
    })
}

fn run_status_of_test_plugin_assert_success_and_parse_json_report() -> Value {
    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &["--format", "json"]);
    assert_exit_code(&output, 0);
    parse_json_report(&output)
}

/// Panics with the last report after `timeout`.
fn poll_status_of_test_plugin_until_predicate_holds(
    timeout: Duration,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let polling_started_at = Instant::now();
    loop {
        let report = run_status_of_test_plugin_assert_success_and_parse_json_report();
        if predicate(&report) {
            return report;
        }
        assert!(
            polling_started_at.elapsed() < timeout,
            "status report did not reach the expected state within {timeout:?}, last report:\n{report:#}"
        );
        thread::sleep(STATUS_POLL_INTERVAL);
    }
}

fn set_default_tier_replicasets_in_test_plugin_topology(replicasets: i64) {
    let topology_path = Path::new(PLUGIN_DIR).join("topology.toml");
    let mut topology: DocumentMut = fs::read_to_string(&topology_path).unwrap().parse().unwrap();
    topology["tier"]["default"]["replicasets"] = toml_edit::value(replicasets);
    fs::write(&topology_path, topology.to_string()).unwrap();
}

fn report_instances(report: &Value) -> &[Value] {
    report["instances"]
        .as_array()
        .unwrap_or_else(|| panic!("no instances in the report:\n{report:#}"))
}

fn find_report_instance_by_dir<'a>(report: &'a Value, dir: &str) -> &'a Value {
    report_instances(report)
        .iter()
        .find(|instance| instance["dir"] == dir)
        .unwrap_or_else(|| panic!("no instance with dir {dir} in the report:\n{report:#}"))
}

/// In the output order.
fn report_instance_dirs(report: &Value) -> Vec<&str> {
    report_instances(report)
        .iter()
        .map(|instance| instance["dir"].as_str().unwrap())
        .collect()
}

fn are_all_instances_in_process_state(report: &Value, process: &str) -> bool {
    report_instances(report)
        .iter()
        .all(|instance| instance["process"] == process)
}

/// `None` unless exactly one instance is marked as the leader.
fn find_single_raft_leader_dir(report: &Value) -> Option<&str> {
    let raft_leader_instances: Vec<&Value> = report_instances(report)
        .iter()
        .filter(|instance| instance["raft_leader"] == true)
        .collect();
    match raft_leader_instances.as_slice() {
        [raft_leader_instance] => raft_leader_instance["dir"].as_str(),
        _ => None,
    }
}

fn make_daemon_run_cmd_args() -> CmdArguments {
    CmdArguments {
        run_args: vec!["--daemon".to_owned()],
        ..Default::default()
    }
}

/// Stops a process with `SIGSTOP` and resumes it with `SIGCONT` on drop,
/// so that a failed test does not leave a frozen instance behind.
struct FrozenProcess(Pid);

impl FrozenProcess {
    fn freeze_with_sigstop(pid: Pid) -> Self {
        kill(pid, Signal::SIGSTOP).expect("failed to send SIGSTOP");
        Self(pid)
    }
}

impl Drop for FrozenProcess {
    fn drop(&mut self) {
        // The process may be gone already; there is nothing to resume then.
        let _ = kill(self.0, Signal::SIGCONT);
    }
}

#[test]
fn test_status_instances_running_cluster() {
    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        make_daemon_run_cmd_args(),
    )
    .expect("failed to start cluster");

    let report =
        poll_status_of_test_plugin_until_predicate_holds(STATUS_STATE_CHANGE_TIMEOUT, |report| {
            let instances = report_instances(report);
            instances.len() == 4
                && instances.iter().all(|instance| {
                    instance["process"] == "running" && instance["current_state"] == "Online"
                })
        });

    let mut instance_count_by_replicaset: BTreeMap<&str, usize> = BTreeMap::new();
    for instance in report_instances(&report) {
        assert!(instance["name"].is_string(), "no name: {instance:#}");
        let replicaset = instance["replicaset"]
            .as_str()
            .unwrap_or_else(|| panic!("no replicaset: {instance:#}"));
        *instance_count_by_replicaset.entry(replicaset).or_default() += 1;
    }
    assert_eq!(
        instance_count_by_replicaset
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [2, 2],
        "expected 2 replicasets of 2 instances: {instance_count_by_replicaset:?}"
    );
    assert!(
        find_single_raft_leader_dir(&report).is_some(),
        "expected exactly one raft leader:\n{report:#}"
    );
    assert_eq!(report["unexpected"], json!([]));

    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &[]);
    assert_exit_code(&output, 0);
    assert_stdout_contains(&output, "4/4 instances running, 0 not started");
    assert_stdout_contains(&output, "★ - raft leader");
}

#[test]
fn test_status_instances_stopped_cluster() {
    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .expect("failed to start cluster");

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);
    // Processes may take a moment to go away after `stop`.
    poll_status_of_test_plugin_until_predicate_holds(STATUS_STATE_CHANGE_TIMEOUT, |report| {
        are_all_instances_in_process_state(report, "stopped")
    });

    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &["--format", "json"]);
    let report_from_plugin_dir = assert_success_without_warning_and_parse_json_report(&output);
    assert_eq!(
        report_instance_dirs(&report_from_plugin_dir),
        ["i1", "i2", "i3", "i4"]
    );
    for instance in report_instances(&report_from_plugin_dir) {
        assert_eq!(instance["process"], "stopped", "{instance:#}");
        assert_eq!(instance["tier"], "default", "{instance:#}");
        assert!(instance["name"].is_string(), "no name: {instance:#}");
        assert!(instance["replicaset"].is_null(), "{instance:#}");
        assert!(instance["current_state"].is_null(), "{instance:#}");
        assert!(instance["raft_leader"].is_null(), "{instance:#}");
    }

    // The cluster dir and `topology.toml` are resolved against `--plugin-path`, not the working dir:
    // a cluster dir resolved against `TESTS_DIR` would not exist and give `not_started` instances.
    let output = run_pike_status_instances(
        Path::new(TESTS_DIR),
        &["--plugin-path", PLUGIN_NAME, "--format", "json"],
    );
    let report_from_tests_dir = assert_success_without_warning_and_parse_json_report(&output);
    assert_eq!(report_from_plugin_dir, report_from_tests_dir);
}

#[test]
fn test_status_instances_stopped_instance() {
    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .expect("failed to start cluster");

    exec_pike([
        "stop",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        "i2",
    ]);

    // The cluster notices the loss of the instance with a delay.
    let report =
        poll_status_of_test_plugin_until_predicate_holds(STATUS_STATE_CHANGE_TIMEOUT, |report| {
            let i2_instance = find_report_instance_by_dir(report, "i2");
            i2_instance["process"] == "stopped" && i2_instance["current_state"] == "Offline"
        });
    for dir in ["i1", "i3", "i4"] {
        assert_eq!(
            find_report_instance_by_dir(&report, dir)["process"],
            "running",
            "{dir}:\n{report:#}"
        );
    }
}

#[test]
fn test_status_instances_raft_leader_stopped() {
    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .expect("failed to start cluster");

    let report =
        poll_status_of_test_plugin_until_predicate_holds(STATUS_STATE_CHANGE_TIMEOUT, |report| {
            find_single_raft_leader_dir(report).is_some()
        });
    let old_leader_dir = find_single_raft_leader_dir(&report).unwrap().to_owned();

    exec_pike([
        "stop",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        &old_leader_dir,
    ]);

    // A new leader is elected after the raft election timeout.
    let report =
        poll_status_of_test_plugin_until_predicate_holds(STATUS_STATE_CHANGE_TIMEOUT, |report| {
            find_single_raft_leader_dir(report)
                .is_some_and(|leader_dir| leader_dir != old_leader_dir)
        });
    assert_eq!(
        find_report_instance_by_dir(&report, &old_leader_dir)["process"],
        "stopped",
        "{report:#}"
    );
}

#[test]
fn test_status_instances_topology_extended() {
    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .expect("failed to start cluster");

    set_default_tier_replicasets_in_test_plugin_topology(3);

    let report = run_status_of_test_plugin_assert_success_and_parse_json_report();
    for dir in ["i5", "i6"] {
        let instance = find_report_instance_by_dir(&report, dir);
        assert_eq!(instance["process"], "not_started", "{instance:#}");
        assert_eq!(instance["tier"], "default", "{instance:#}");
        assert!(instance["name"].is_null(), "{instance:#}");
        assert_eq!(instance["raft_leader"], false, "{instance:#}");
    }
    for dir in ["i1", "i2", "i3", "i4"] {
        assert_eq!(
            find_report_instance_by_dir(&report, dir)["process"],
            "running",
            "{dir}:\n{report:#}"
        );
    }
    assert_eq!(report_instances(&report).len(), 6, "{report:#}");
    assert_eq!(report["unexpected"], json!([]));
}

#[test]
fn test_status_instances_topology_reduced() {
    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .expect("failed to start cluster");

    set_default_tier_replicasets_in_test_plugin_topology(1);

    let report = run_status_of_test_plugin_assert_success_and_parse_json_report();
    assert_eq!(report["unexpected"], json!(["i3", "i4"]), "{report:#}");
    for dir in ["i3", "i4"] {
        let instance = find_report_instance_by_dir(&report, dir);
        assert_eq!(instance["process"], "running", "{instance:#}");
        // Taken from the cluster: `topology.toml` knows nothing about these instances.
        assert_eq!(instance["tier"], "default", "{instance:#}");
    }

    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &[]);
    assert_exit_code(&output, 0);
    assert_stdout_contains(&output, "2 not described in topology.toml: i3, i4");
}

#[test]
fn test_status_instances_no_cluster_dir() {
    init_plugin(PLUGIN_NAME);
    assert!(
        !Path::new(CLUSTER_DIR).exists(),
        "a new plugin must have no cluster dir: {CLUSTER_DIR}"
    );

    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &["--format", "json"]);
    let report = assert_success_without_warning_and_parse_json_report(&output);
    assert_eq!(report_instance_dirs(&report), ["i1", "i2", "i3", "i4"]);
    assert!(
        are_all_instances_in_process_state(&report, "not_started"),
        "{report:#}"
    );
}

#[test]
fn test_status_instances_without_topology() {
    init_plugin(PLUGIN_NAME);
    fs::remove_file(Path::new(PLUGIN_DIR).join("topology.toml")).unwrap();

    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &[]);
    assert_exit_code(&output, 1);
    assert_stdout_contains(&output, "It seems to me, that you are trying to");
}

#[test]
fn test_status_instances_frozen_instance() {
    /// `CLUSTER_INSTANCES_QUERY_TIMEOUT` of the command (10 s) with a margin for the rest of its work.
    const STATUS_DURATION_LIMIT_WITH_FROZEN_INSTANCE: Duration = Duration::from_secs(20);

    let _cluster = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        make_daemon_run_cmd_args(),
    )
    .expect("failed to start cluster");

    // `i1` is asked first; a frozen process still accepts connections on `admin.sock`
    // (the kernel queues them), but never answers.
    let i1_pid_file_contents =
        fs::read_to_string(Path::new(CLUSTER_DIR).join("i1").join("pid")).unwrap();
    let i1_pid = Pid::from_raw(i1_pid_file_contents.trim().parse().unwrap());
    // Declared after `_cluster`, so the instance is resumed before the cluster is stopped.
    let _frozen_i1 = FrozenProcess::freeze_with_sigstop(i1_pid);

    let status_started_at = Instant::now();
    let output = run_pike_status_instances(Path::new(PLUGIN_DIR), &["--format", "json"]);
    let status_duration = status_started_at.elapsed();

    assert_exit_code(&output, 0);

    assert!(
        status_duration < STATUS_DURATION_LIMIT_WITH_FROZEN_INSTANCE,
        "status took {status_duration:?}\n{}",
        describe_exit_status_stdout_and_stderr(&output)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(CLUSTER_DATA_UNAVAILABLE_WARNING) && stderr.contains("no response from"),
        "{}",
        describe_exit_status_stdout_and_stderr(&output)
    );
    let report = parse_json_report(&output);
    assert_eq!(report_instance_dirs(&report), ["i1", "i2", "i3", "i4"]);
    for instance in report_instances(&report) {
        assert_eq!(instance["process"], "running", "{instance:#}");
        assert!(instance["replicaset"].is_null(), "{instance:#}");
    }
}

#[test]
fn test_status_instances_closed_stdout() {
    init_plugin(PLUGIN_NAME);

    // Writing to a pipe without a reader fails with EPIPE; Rust ignores SIGPIPE,
    // so pike gets the error instead of being killed by the signal.
    let (pipe_reader, pipe_writer) = std::io::pipe().unwrap();
    drop(pipe_reader);
    let output = make_pike_status_instances_command(Path::new(PLUGIN_DIR), &[])
        .stdout(pipe_writer)
        .output()
        .expect("failed to execute pike status instances");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_exit_code(&output, 1);
    assert!(!stderr.contains("panicked"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("failed to write instances to stdout"),
        "stderr:\n{stderr}"
    );
}
