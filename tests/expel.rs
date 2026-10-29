mod helpers;

use helpers::{
    exec_pike, get_picodata_query_output, is_instance_running, run_cluster, CmdArguments,
    CLUSTER_DIR, PLUGIN_NAME, TESTS_DIR,
};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const TOTAL_INSTANCES: i32 = 4;
const CLUSTER_START_TIMEOUT: Duration = Duration::from_secs(120);
const INSTANCE_STOP_TIMEOUT: Duration = Duration::from_secs(60);

fn wait_instance_stopped(instance_dir: &Path) {
    let start = Instant::now();
    while is_instance_running(instance_dir) {
        assert!(
            start.elapsed() < INSTANCE_STOP_TIMEOUT,
            "Timeout has reached. Instance was not stopped."
        );
        thread::sleep(Duration::from_millis(200));
    }
}

fn try_expel<const N: usize>(args: [&str; N]) -> bool {
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let mut pike_child = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .arg("expel")
        .args(args)
        .args(["--plugin-path", PLUGIN_NAME])
        .current_dir(TESTS_DIR)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to execute pike");

    // Pike leaves a supervisor process holding the pipes until the parent is reaped,
    // so wait for the exit status first and only then read the output.
    let status = pike_child.wait().unwrap();

    let outputs: [Box<dyn Read + Send>; 2] = [
        Box::new(pike_child.stdout.unwrap()),
        Box::new(pike_child.stderr.unwrap()),
    ];
    for output in outputs {
        for line in BufReader::new(output).lines() {
            println!("{}", line.expect("failed to read pike output"));
        }
    }

    status.success()
}

fn picodata_name(dir_name: &str) -> String {
    let cluster_dir = Path::new(CLUSTER_DIR);

    fs::read_dir(cluster_dir)
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

fn current_state(picodata_name: &str) -> String {
    get_picodata_query_output(
        Path::new(CLUSTER_DIR),
        &format!("SELECT current_state FROM _pico_instance WHERE name = '{picodata_name}';"),
    )
}

fn wait_instance_offline(picodata_name: &str) {
    let start = Instant::now();
    loop {
        let state = current_state(picodata_name);
        if state.contains("Offline") {
            return;
        }

        assert!(
            start.elapsed() < INSTANCE_STOP_TIMEOUT,
            "Timeout has reached. Instance '{picodata_name}' did not become offline: {state}"
        );
        thread::sleep(Duration::from_millis(500));
    }
}

fn assert_instance_expelled(picodata_name: &str) {
    let state = current_state(picodata_name);
    assert!(
        state.contains("Expelled"),
        "instance '{picodata_name}' is not expelled: {state}"
    );
}

#[test]
fn test_pike_expel_running_instance_with_force() {
    let target_instance = "i2";
    let cluster_dir = Path::new(CLUSTER_DIR);

    let _cluster_handle = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    let picodata_name = picodata_name(target_instance);

    exec_pike([
        "expel",
        &picodata_name,
        "--force",
        "--plugin-path",
        PLUGIN_NAME,
    ]);

    wait_instance_stopped(&cluster_dir.join(target_instance));
    assert_instance_expelled(&picodata_name);

    for instance_name in ["i1", "i3", "i4"] {
        assert!(
            is_instance_running(&cluster_dir.join(instance_name)),
            "instance '{instance_name}' was stopped by expel of '{target_instance}'"
        );
    }
}

#[test]
fn test_pike_expel_running_instance_without_force_fails() {
    let target_instance = "i2";
    let cluster_dir = Path::new(CLUSTER_DIR);

    let _cluster_handle = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    let picodata_name = picodata_name(target_instance);

    // Running instance can only be expelled forcibly. Short timeout, since
    // picodata waits for the whole period before giving up.
    assert!(
        !try_expel([&picodata_name, "--timeout", "5"]),
        "expel of a running instance without --force must fail"
    );

    assert!(is_instance_running(&cluster_dir.join(target_instance)));
}

#[test]
fn test_pike_expel_stopped_instance() {
    let target_instance = "i3";
    let cluster_dir = Path::new(CLUSTER_DIR);

    let _cluster_handle = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    exec_pike(["stop", "-i", target_instance, "--plugin-path", PLUGIN_NAME]);
    wait_instance_stopped(&cluster_dir.join(target_instance));

    let picodata_name = picodata_name(target_instance);
    wait_instance_offline(&picodata_name);

    exec_pike(["expel", &picodata_name, "--plugin-path", PLUGIN_NAME]);

    assert_instance_expelled(&picodata_name);
}

#[test]
fn test_pike_expel_invalid_instance() {
    let cluster_dir = Path::new(CLUSTER_DIR);

    let _cluster_handle = run_cluster(
        CLUSTER_START_TIMEOUT,
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    // There is no such instance.
    assert!(!try_expel(["unknown_instance"]));

    // The cluster is not affected.
    for instance_name in ["i1", "i2", "i3", "i4"] {
        assert!(is_instance_running(&cluster_dir.join(instance_name)));
    }
}
