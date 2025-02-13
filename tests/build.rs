mod helpers;

use helpers::{build_plugin, check_plugin_version_artefacts, exec_pike, PLUGIN_DIR, TESTS_DIR};
use std::{
    fs::{self},
    path::Path,
    vec,
};

#[test]
fn test_cargo_build() {
    // Cleaning up metadata from past run
    if Path::new(PLUGIN_DIR).exists() {
        fs::remove_dir_all(PLUGIN_DIR).unwrap();
    }

    assert!(
        exec_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR, &vec![])
            .unwrap()
            .success()
    );

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
