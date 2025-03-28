mod helpers;

use helpers::{assert_plugin_build_artefacts, build_plugin, cleanup_dir, exec_pike, PLUGIN_DIR};
use std::path::Path;

#[test]
fn test_cargo_build() {
    let plugin_path = Path::new(PLUGIN_DIR);
    cleanup_dir(plugin_path);

    exec_pike(["plugin", "new", "test-plugin"]);

    build_plugin(&helpers::BuildType::Debug, "0.1.0");
    build_plugin(&helpers::BuildType::Debug, "0.1.1");

    let build_path = plugin_path.join("target").join("debug").join("test-plugin");
    assert_plugin_build_artefacts(&build_path.join("0.1.0"), false);
    assert_plugin_build_artefacts(&build_path.join("0.1.1"), true);

    build_plugin(&helpers::BuildType::Release, "0.1.0");
    build_plugin(&helpers::BuildType::Release, "0.1.1");

    let build_path = plugin_path
        .join("target")
        .join("release")
        .join("test-plugin");
    assert_plugin_build_artefacts(&build_path.join("0.1.0"), false);
    assert_plugin_build_artefacts(&build_path.join("0.1.1"), true);
}
