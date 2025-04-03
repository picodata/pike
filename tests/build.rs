mod helpers;

use helpers::{assert_plugin_build_artefacts, build_plugin, init_plugin};
use std::path::Path;

#[test]
fn test_cargo_build() {
    let plugin_path = Path::new("./tests/tmp/test-plugin-build");

    init_plugin("test-plugin-build", vec![]);

    build_plugin(&helpers::BuildType::Debug, "0.1.0", plugin_path);
    build_plugin(&helpers::BuildType::Debug, "0.1.1", plugin_path);

    let build_path = plugin_path
        .join("target")
        .join("debug")
        .join("test-plugin-build");
    assert_plugin_build_artefacts(&build_path.join("0.1.0"), false);
    assert_plugin_build_artefacts(&build_path.join("0.1.1"), true);

    build_plugin(&helpers::BuildType::Release, "0.1.0", plugin_path);
    build_plugin(&helpers::BuildType::Release, "0.1.1", plugin_path);

    let build_path = plugin_path
        .join("target")
        .join("release")
        .join("test-plugin-build");
    assert_plugin_build_artefacts(&build_path.join("0.1.0"), false);
    assert_plugin_build_artefacts(&build_path.join("0.1.1"), true);
}
