mod helpers;

use helpers::{
    build_plugin, check_plugin_version_artefacts, run_cluster, run_pike, wait_for_proc, PLUGIN_DIR,
    TESTS_DIR,
};
use std::{fs, path::Path, time::Duration};

const TOTAL_INSTANCES: i32 = 4;
#[test]
fn test_cluster_setup_debug() {
    let _cluster_handle = run_cluster(Duration::from_secs(120), TOTAL_INSTANCES).unwrap();
}

#[test]
fn test_cargo_build() {
    // Cleaning up metadata from past run
    if Path::new(PLUGIN_DIR).exists() {
        fs::remove_dir_all(PLUGIN_DIR).unwrap();
    }

    let mut plugin_creation_proc =
        run_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR).unwrap();

    wait_for_proc(&mut plugin_creation_proc, Duration::from_secs(10));

    build_plugin(&helpers::BuildType::Debug, "0.1.0");
    build_plugin(&helpers::BuildType::Debug, "0.1.1");
    build_plugin(&helpers::BuildType::Release, "0.1.0");
    build_plugin(&helpers::BuildType::Release, "0.1.1");

    assert!(check_plugin_version_artefacts(
        &Path::new(PLUGIN_DIR)
            .join("target")
            .join("debug")
            .join("test-plugin")
            .join("0.1.0"),
        false
    ));

    assert!(check_plugin_version_artefacts(
        &Path::new(PLUGIN_DIR)
            .join("target")
            .join("debug")
            .join("test-plugin")
            .join("0.1.1"),
        true
    ));

    assert!(check_plugin_version_artefacts(
        &Path::new(PLUGIN_DIR)
            .join("target")
            .join("release")
            .join("test-plugin")
            .join("0.1.0"),
        false
    ));

    assert!(check_plugin_version_artefacts(
        &Path::new(PLUGIN_DIR)
            .join("target")
            .join("release")
            .join("test-plugin")
            .join("0.1.1"),
        true
    ));
}
