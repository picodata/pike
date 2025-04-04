mod helpers;

use helpers::{cleanup_dir, exec_pike, PLUGIN_DIR};
use std::fs;
use std::path::Path;

#[test]
fn test_cargo_plugin_new() {
    let root_dir = Path::new(PLUGIN_DIR);
    cleanup_dir(root_dir);

    // Test creating simple plugin
    exec_pike(["plugin", "new", "test-plugin"]);

    assert!(root_dir.join("picodata.yaml").exists());
    assert!(root_dir.join(".git").exists());
    assert!(root_dir.join("plugin_config.yaml").exists());
    assert!(root_dir.join("manifest.yaml.template").exists());

    cleanup_dir(root_dir);

    // Test creating plugin without git
    exec_pike(["plugin", "new", "test-plugin", "--without-git"]);

    assert!(!root_dir.join(".git").exists());

    cleanup_dir(root_dir);

    // Test creating plugin as workspace
    exec_pike(["plugin", "new", "test-plugin", "--workspace"]);

    let subcrate_path = Path::new(PLUGIN_DIR).join("test-plugin");
    assert!(subcrate_path.exists());

    assert!(root_dir.join(".cargo").join("config.toml").exists());
    assert!(!subcrate_path.join(".cargo").exists());

    assert!(root_dir.join("picodata.yaml").exists());
    assert!(!subcrate_path.join("picodata.yaml").exists());

    assert!(root_dir.join("topology.toml").exists());
    assert!(!subcrate_path.join("topology.toml").exists());

    assert!(root_dir.join(".git").exists());
    assert!(!subcrate_path.join(".git").exists());

    assert!(root_dir.join(".gitignore").exists());
    assert!(!subcrate_path.join(".gitignore").exists());

    assert!(root_dir.join("rust-toolchain.toml").exists());
    assert!(!subcrate_path.join("rust-toolchain.toml").exists());

    assert!(root_dir.join("tmp").exists());
    assert!(!subcrate_path.join("tmp").exists());

    let contents = fs::read_to_string(root_dir.join("Cargo.toml")).unwrap();
    assert!(contents.contains("[workspace]"));
}
