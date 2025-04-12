mod helpers;
use helpers::{run_cluster, CmdArguments, PLUGIN_NAME, TESTS_DIR};
use std::{
    io::Read,
    process::{Command, Stdio},
    time::Duration,
};

const TOTAL_INSTANCES: i32 = 4;

#[test]
fn test_enter_instance() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let mut pike_child = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .args(vec!["enter", "i1", "--plugin-path", PLUGIN_NAME])
        .current_dir(TESTS_DIR)
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .expect("failed to execute pike");

    let stdin = pike_child.stdin.take().expect("Failed to open stdin");
    let mut stdout = pike_child.stdout.take().expect("Failed to open stdout");

    // Here we are dropping stdin, which is equal to sending Ctrl+D singnal to pike enter proccess
    drop(stdin);
    let status = pike_child.wait().unwrap();
    assert!(
        status.success(),
        "Cluster failed while handling enter command"
    );

    // Capture the output and check successfull connection to picodata console
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .expect("Failed to read output");

    assert!(
        output.contains("Connected to admin console by socket path"),
        "Failed to enter picodata instance"
    );
}
