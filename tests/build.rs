mod helpers;

use helpers::{assert_path_existance, build_plugin, init_plugin, validate_symlink, LIB_EXT};
use std::path::Path;

#[test]
fn test_cargo_build() {
    let plugin_path = Path::new("./tests/tmp/test-plugin-build");

    init_plugin("test-plugin-build");

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

fn assert_plugin_build_artefacts(plugin_path: &Path, must_be_symlinks: bool) {
    let lib_path = plugin_path.join(format!("libtest_plugin_build.{LIB_EXT}"));

    if must_be_symlinks {
        assert!(validate_symlink(&lib_path));
    }

    assert_path_existance(&plugin_path.join("manifest.yaml"), false);
    assert_path_existance(&lib_path, must_be_symlinks);
    assert_path_existance(&plugin_path.join("migrations"), false);
}
