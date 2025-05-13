mod helpers;

use helpers::{exec_pike, init_plugin};
use std::{fs, path::Path, process::Command};

#[test]
fn test_plugin_run_clippy() {
    let plugin_path = Path::new("./tests/tmp/plugin-template-tests");
    init_plugin("plugin-template-tests");

    let output = Command::new("cargo")
        .args([
            "clippy",
            "--all-features",
            "--lib",
            "--examples",
            "--tests",
            "--benches",
            "--",
            "-W",
            "clippy::all",
            "-W",
            "clippy::pedantic",
            "-D",
            "warnings",
        ])
        .current_dir(plugin_path)
        .output()
        .expect("Clippy run error");

    assert!(
        output.status.success(),
        "Clippy found errors:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_plugin_run_tests() {
    // We make this kludge for fix "No buffer space available" error
    // that occurs due to the socket path being too long.
    let tmp_test_dir = Path::new("/tmp/pike-tests");
    let _ = fs::remove_dir_all(tmp_test_dir);
    fs::create_dir(tmp_test_dir).unwrap();
    let plugin_path = tmp_test_dir.join("plugin-template-tests");

    exec_pike(["plugin", "new", plugin_path.to_str().unwrap()]);

    let output = Command::new("cargo")
        .arg("test")
        .current_dir(plugin_path)
        .output()
        .expect("Cargo run error");

    assert!(
        output.status.success(),
        "Cargo tests failed:\n\n{}\n\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
