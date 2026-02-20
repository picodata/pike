mod helpers;

use helpers::{exec_pike, init_plugin, init_plugin_workspace, LIB_EXT, TESTS_DIR};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const PACK_PLUGIN_NAME: &str = "test-pack-plugin";
const VERSION: &str = "0.1.0";

const ROLLING: &[&str] = &[
    "arch",
    "gentoo",
    "void",
    "opensuse-tumbleweed",
    "artix",
    "manjaro",
    "endeavouros",
    "garuda",
    "kaos",
];

fn find_archive(dir: &Path, name: &str, version: &str) -> PathBuf {
    let prefix = format!("{name}_{version}-");
    let mut matches = vec![];
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with(&prefix) && file_name.ends_with(".tar.gz") {
            matches.push(entry.path());
        }
    }
    assert!(
        !matches.is_empty(),
        "No archive found in {} with prefix {prefix}",
        dir.display()
    );
    assert_eq!(
        matches.len(),
        1,
        "Expected exactly one archive, found {}: {:?}",
        matches.len(),
        matches
    );
    matches.remove(0)
}

fn has_legacy_archive(dir: &Path, name: &str, version: &str) -> bool {
    dir.join(format!("{name}-{version}.tar.gz")).exists()
}

fn assert_no_legacy_archive(dir: &Path, name: &str, version: &str) {
    assert!(
        !has_legacy_archive(dir, name, version),
        "Legacy archive <{name}-{version}.tar.gz> must NOT be produced anymore (found in {})",
        dir.display()
    );
}

fn assert_os_suffix(file_name: &str, name: &str, version: &str) -> (String, String) {
    // name_version-<osid>_<variant>.tar.gz
    let prefix = format!("{name}_{version}-");
    assert!(
        file_name.starts_with(&prefix),
        "Archive name {file_name} must start with {prefix}"
    );
    let rest = &file_name[prefix.len()..file_name.len() - ".tar.gz".len()];
    let parts: Vec<&str> = rest.split('_').collect();
    assert!(
        parts.len() >= 2,
        "OS suffix must contain at least one underscore: got {rest}"
    );
    assert!(
        parts.iter().all(|p| !p.is_empty()),
        "OS suffix parts must be non-empty: {rest}"
    );

    // Базовая валидация алфавита
    let allowed = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".-_".contains(c))
    };
    assert!(
        allowed(rest),
        "OS suffix '{rest}' contains unsupported chars (allowed: a-z0-9._-)"
    );

    let os_id = parts[0].to_string();
    let variant = parts[1..].join("_");

    (os_id, variant)
}

#[test]
fn test_cargo_pack() {
    init_plugin(PACK_PLUGIN_NAME);

    exec_pike([
        "plugin",
        "pack",
        "--plugin-path",
        PACK_PLUGIN_NAME,
        "--target-dir",
        "tmp_target",
    ]);

    // Hail for archive handling in Rust
    let plugin_path = Path::new(TESTS_DIR)
        .join(PACK_PLUGIN_NAME)
        .join("tmp_target")
        .join("release");

    let archive_path = find_archive(&plugin_path, PACK_PLUGIN_NAME, VERSION);
    let file_name = archive_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _ = assert_os_suffix(&file_name, PACK_PLUGIN_NAME, VERSION);
    assert_no_legacy_archive(&plugin_path, PACK_PLUGIN_NAME, VERSION);

    helpers::unpack_archive(&archive_path, &plugin_path);

    let base_file_path = plugin_path.join(PACK_PLUGIN_NAME).join(VERSION);
    assert!(base_file_path
        .join(format!("libtest_pack_plugin.{LIB_EXT}"))
        .exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("migrations").is_dir());
}

#[test]
fn test_cargo_pack_assets() {
    let pack_plugin_path = Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME);

    init_plugin(PACK_PLUGIN_NAME);

    // Change build script for sub plugin to test custom assets
    fs::copy(
        Path::new(TESTS_DIR)
            .parent()
            .unwrap()
            .join("assets")
            .join("custom_assets_build.rs"),
        pack_plugin_path.join("build.rs"),
    )
    .unwrap();

    // release build
    exec_pike(["plugin", "pack", "--plugin-path", PACK_PLUGIN_NAME]);

    // check release archive
    let release_dir = pack_plugin_path.join("target").join("release");
    let release_archive = find_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);
    let release_archive_name = release_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _ = assert_os_suffix(&release_archive_name, PACK_PLUGIN_NAME, VERSION);
    assert_no_legacy_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);

    let unzipped_release = pack_plugin_path.join("unzipped_release");
    let base_file_path = unzipped_release.join(PACK_PLUGIN_NAME).join(VERSION);

    helpers::unpack_archive(&release_archive, &unzipped_release);

    assert!(base_file_path
        .join(format!("libtest_pack_plugin.{LIB_EXT}"))
        .exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("plugin_config.yaml").exists());
    let mig_file = base_file_path.join("migrations").join("0001_init.sql");
    let mig_file_content = fs::read_to_string(&mig_file).unwrap();
    assert!(!mig_file_content.contains("-- test"));

    // debug build
    exec_pike([
        "plugin",
        "pack",
        "--plugin-path",
        PACK_PLUGIN_NAME,
        "--debug",
    ]);

    // check debug archive
    let debug_dir = pack_plugin_path.join("target").join("debug");
    let debug_archive = find_archive(&debug_dir, PACK_PLUGIN_NAME, VERSION);
    let debug_archive_name = debug_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _ = assert_os_suffix(&debug_archive_name, PACK_PLUGIN_NAME, VERSION);
    assert_no_legacy_archive(&debug_dir, PACK_PLUGIN_NAME, VERSION);

    let unzipped_debug = pack_plugin_path.join("unzipped_debug");
    let base_file_path = unzipped_debug.join(PACK_PLUGIN_NAME).join(VERSION);

    helpers::unpack_archive(&debug_archive, &unzipped_debug);

    assert!(base_file_path
        .join(format!("libtest_pack_plugin.{LIB_EXT}"))
        .exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("plugin_config.yaml").exists());
    let mig_file = base_file_path.join("migrations").join("0001_init.sql");
    let mig_file_content = fs::read_to_string(&mig_file).unwrap();
    assert!(!mig_file_content.contains("-- test"));

    // mutate assets
    let mut source_mig_file = OpenOptions::new()
        .append(true)
        .open(pack_plugin_path.join("migrations").join("0001_init.sql"))
        .unwrap();
    writeln!(source_mig_file, "-- test").unwrap();
    let mut source_config_file = OpenOptions::new()
        .append(true)
        .open(pack_plugin_path.join("plugin_config.yaml"))
        .unwrap();
    writeln!(source_config_file, "# test").unwrap();

    exec_pike(["plugin", "pack", "--plugin-path", PACK_PLUGIN_NAME]);

    let release_dir = pack_plugin_path.join("target").join("release");
    let release_archive = find_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);
    let release_archive_name = release_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _ = assert_os_suffix(&release_archive_name, PACK_PLUGIN_NAME, VERSION);

    let unzipped_changed = pack_plugin_path.join("unzipped_release_with_changed_assets");
    helpers::unpack_archive(&release_archive, &unzipped_changed);

    let changed_base = unzipped_changed.join(PACK_PLUGIN_NAME).join(VERSION);
    let changed_mig_file = changed_base.join("migrations").join("0001_init.sql");
    let mig_file_content = fs::read_to_string(&changed_mig_file).unwrap();
    assert!(mig_file_content.contains("-- test"));
    let config_file = changed_base.join("plugin_config.yaml");
    let config_content = fs::read_to_string(&config_file).unwrap();
    assert!(config_content.contains("# test"));
}

#[test]
fn test_custom_assets_with_targets() {
    let tests_dir = Path::new(TESTS_DIR);
    let plugin_path = tests_dir.join(PACK_PLUGIN_NAME);

    init_plugin(PACK_PLUGIN_NAME);

    // Change build script for plugin to test custom assets
    fs::copy(
        tests_dir.join("../assets/custom_assets_with_targets_build.rs"),
        plugin_path.join("build.rs"),
    )
    .unwrap();

    // Fully test pack command for proper artifacts inside archives
    exec_pike([
        "plugin",
        "pack",
        "--debug",
        "--plugin-path",
        PACK_PLUGIN_NAME,
    ]);

    // Check the debug archive
    let debug_dir = plugin_path.join("target").join("debug");
    let debug_archive = find_archive(&debug_dir, PACK_PLUGIN_NAME, VERSION);
    let debug_archive_name = debug_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _ = assert_os_suffix(&debug_archive_name, PACK_PLUGIN_NAME, VERSION);
    assert_no_legacy_archive(&debug_dir, PACK_PLUGIN_NAME, VERSION);

    let unzipped_debug = plugin_path.join("unzipped_debug");
    helpers::unpack_archive(&debug_archive, &unzipped_debug);

    let assets_file_path = unzipped_debug.join(PACK_PLUGIN_NAME).join(VERSION);

    assert!(assets_file_path.join("plugin_config.yaml").exists());
    assert!(assets_file_path.join("not.cargo").exists());
    assert!(assets_file_path
        .join("other")
        .join("name")
        .join("Cargo.unlock")
        .exists());
    assert!(assets_file_path
        .join("other")
        .join("name")
        .join("lib.rs")
        .exists());

    exec_pike(["plugin", "pack", "--plugin-path", PACK_PLUGIN_NAME]);

    // Check the release archive
    let release_dir = plugin_path.join("target").join("release");
    let release_archive = find_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);
    let release_archive_name = release_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let _ = assert_os_suffix(&release_archive_name, PACK_PLUGIN_NAME, VERSION);
    assert_no_legacy_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);

    let unzipped_release = plugin_path.join("unzipped_release");
    helpers::unpack_archive(&release_archive, &unzipped_release);

    let assets_file_path = unzipped_release.join(PACK_PLUGIN_NAME).join(VERSION);

    assert!(assets_file_path.join("plugin_config.yaml").exists());
    assert!(assets_file_path.join("not.cargo").exists());
    assert!(assets_file_path
        .join("other")
        .join("name")
        .join("Cargo.unlock")
        .exists());
    assert!(assets_file_path
        .join("other")
        .join("name")
        .join("lib.rs")
        .exists());
}

#[test]
fn test_no_legacy_archive_name() {
    init_plugin(PACK_PLUGIN_NAME);

    exec_pike(["plugin", "pack", "--plugin-path", PACK_PLUGIN_NAME]);

    let release_dir = Path::new(TESTS_DIR)
        .join(PACK_PLUGIN_NAME)
        .join("target")
        .join("release");

    assert_no_legacy_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);

    // Убеждаемся, что новый архив есть
    let new_archive = find_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);
    assert!(
        new_archive.exists(),
        "Expected new archive at {}",
        new_archive.display()
    );
    let new_name = new_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let (_os_id, _variant) = assert_os_suffix(&new_name, PACK_PLUGIN_NAME, VERSION);
}

#[test]
fn test_workspace_pack_multiple_archives() {
    const WS_NAME: &str = "ws-pack";
    const SUB: &str = "sub_plugin_ws";

    init_plugin_workspace(WS_NAME);

    exec_pike(["plugin", "add", SUB, "--plugin-path", WS_NAME]);

    exec_pike(["plugin", "pack", "--plugin-path", WS_NAME]);

    let release_dir = Path::new(TESTS_DIR)
        .join(WS_NAME)
        .join("target")
        .join("release");

    let ws_archive = find_archive(&release_dir, WS_NAME, VERSION);
    let ws_file_name = ws_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let (ws_id, ws_variant) = assert_os_suffix(&ws_file_name, WS_NAME, VERSION);
    assert!(
        !ws_id.is_empty() && !ws_variant.is_empty(),
        "workspace root OS suffix parts must be non-empty"
    );

    let sub_archive = find_archive(&release_dir, SUB, VERSION);
    let sub_file_name = sub_archive
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let (_sub_id, _sub_variant) = assert_os_suffix(&sub_file_name, SUB, VERSION);

    assert_no_legacy_archive(&release_dir, WS_NAME, VERSION);
    assert_no_legacy_archive(&release_dir, SUB, VERSION);

    let unpack_dir = release_dir.join("unpacked_ws");
    fs::create_dir_all(&unpack_dir).unwrap();
    helpers::unpack_archive(&ws_archive, &unpack_dir);
    let base = unpack_dir.join(WS_NAME).join(VERSION);

    let expected_lib = format!("lib{}.{LIB_EXT}", WS_NAME.replace('-', "_"));
    assert!(
        base.join(&expected_lib).exists(),
        "Expected library {expected_lib} in workspace root plugin archive (checked path {})",
        base.display()
    );
    assert!(base.join("manifest.yaml").exists());
    assert!(base.join("migrations").is_dir());

    let unpack_sub_dir = release_dir.join("unpacked_sub");
    fs::create_dir_all(&unpack_sub_dir).unwrap();
    helpers::unpack_archive(&sub_archive, &unpack_sub_dir);
    let sub_base = unpack_sub_dir.join(SUB).join(VERSION);
    let expected_sub_lib = format!("lib{}.{LIB_EXT}", SUB.replace('-', "_"));
    assert!(
        sub_base.join(&expected_sub_lib).exists(),
        "Expected subplugin library {expected_sub_lib} (checked path {})",
        sub_base.display()
    );
    assert!(sub_base.join("manifest.yaml").exists());
    assert!(sub_base.join("migrations").is_dir());
}

#[test]
fn test_os_suffix_semantics_rolling_or_variant() {
    init_plugin(PACK_PLUGIN_NAME);

    exec_pike(["plugin", "pack", "--plugin-path", PACK_PLUGIN_NAME]);

    let release_dir = Path::new(TESTS_DIR)
        .join(PACK_PLUGIN_NAME)
        .join("target")
        .join("release");

    let archive_path = find_archive(&release_dir, PACK_PLUGIN_NAME, VERSION);
    let file_name = archive_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let (os_id, variant) = assert_os_suffix(&file_name, PACK_PLUGIN_NAME, VERSION);

    if ROLLING.contains(&os_id.as_str()) {
        assert_eq!(
            variant, "rolling",
            "Rolling distro id '{os_id}' must have variant 'rolling', got '{variant}'"
        );
    } else {
        assert!(
            !variant.is_empty(),
            "Variant for non-rolling distro '{os_id}' must not be empty"
        );
    }
}
