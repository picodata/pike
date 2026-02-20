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

#[test]
fn test_picodata_config_template_rendering() {
    let plugin_path = Path::new("./tests/tmp/plugin-template-config-test");
    init_plugin("plugin-template-config-test");

    // Create a picodata.yaml config template with Liquid variables
    let config_path = plugin_path.join("picodata.yaml");
    let template_content = r#"cluster:
  name: "test-cluster"
  tier:
    default:
      can_vote: true
  default_replication_factor: 2

instance:
  name: "instance-{{ instance_id }}"
  log:
    level: warn
    format: plain
  memtx:
    memory: 67108864
"#;

    fs::write(&config_path, template_content).expect("Failed to write templated picodata.yaml");

    let config_content = fs::read_to_string(&config_path).expect("Failed to read picodata.yaml");

    // Verify that the template contains Liquid template variables
    assert!(
        config_content.contains("{{ instance_id }}"),
        "picodata.yaml should contain {{ instance_id }} template variable"
    );

    // Test template rendering with Liquid
    let parser = liquid::ParserBuilder::with_stdlib()
        .build()
        .expect("Failed to build Liquid parser");

    let template = parser
        .parse(&config_content)
        .expect("Failed to parse picodata.yaml as Liquid template");

    // Render with instance_id = 1
    let ctx = liquid::object!({
        "instance_id": 1,
    });
    let rendered = template
        .render(&ctx)
        .expect("Failed to render template with instance_id=1");

    // Verify that the rendered output contains the substituted value
    assert!(
        rendered.contains("instance-1"),
        "Rendered config should contain 'instance-1', got:\n{rendered}"
    );
    assert!(
        !rendered.contains("{{ instance_id }}"),
        "Rendered config should not contain template variable, got:\n{rendered}"
    );

    // Render with instance_id = 42
    let ctx = liquid::object!({
        "instance_id": 42,
    });
    let rendered = template
        .render(&ctx)
        .expect("Failed to render template with instance_id=42");

    assert!(
        rendered.contains("instance-42"),
        "Rendered config should contain 'instance-42', got:\n{rendered}"
    );
    assert!(
        !rendered.contains("{{ instance_id }}"),
        "Rendered config should not contain template variable, got:\n{rendered}"
    );
}
