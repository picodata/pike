mod helpers;

use helpers::{cleanup_dir, exec_pike, TESTS_DIR};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
};

pub const PACK_PLUGIN_NAME: &str = "test-pack-plugin";

#[test]
fn test_cargo_pack() {
    cleanup_dir(&Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME));

    exec_pike(vec!["plugin", "new", PACK_PLUGIN_NAME], TESTS_DIR, &vec![]);

    exec_pike(
        vec!["plugin", "pack"],
        Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME),
        &vec!["--target-dir".to_string(), "tmp_target".to_string()],
    );

    // Hail for archive handling in Rust
    let plugin_path = Path::new(TESTS_DIR)
        .join(PACK_PLUGIN_NAME)
        .join("tmp_target");
    helpers::unpack_archive(
        &plugin_path
            .join("release")
            .join("test_pack_plugin-0.1.0.tar.gz"),
        &plugin_path,
    );

    let base_file_path = plugin_path.join("test_pack_plugin").join("0.1.0");
    assert!(base_file_path.join("libtest_pack_plugin.so").exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("migrations").is_dir());
}

#[test]
fn test_cargo_pack_assets() {
    let pack_plugin_path = Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME);
    cleanup_dir(&pack_plugin_path);

    exec_pike(vec!["plugin", "new", PACK_PLUGIN_NAME], TESTS_DIR, &vec![]);

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
    exec_pike(
        vec!["plugin", "pack"],
        Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME),
        &vec![],
    );

    // check release archive
    let unzipped_dir = pack_plugin_path.join("unzipped_release");
    let base_file_path = unzipped_dir.join("test_pack_plugin").join("0.1.0");

    helpers::unpack_archive(
        &pack_plugin_path
            .join("target")
            .join("release")
            .join("test_pack_plugin-0.1.0.tar.gz"),
        &unzipped_dir,
    );

    assert!(base_file_path.join("libtest_pack_plugin.so").exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("plugin_config.yaml").exists());
    let mig_file = base_file_path.join("migrations").join("0001_init.sql");
    let mig_file_content = fs::read_to_string(&mig_file).unwrap();
    assert!(!mig_file_content.contains("-- test"));

    // debug build
    exec_pike(
        vec!["plugin", "pack"],
        Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME),
        &vec!["--debug".into()],
    );

    // check debug archive
    let unzipped_dir = pack_plugin_path.join("unzipped_debug");
    let base_file_path = unzipped_dir.join("test_pack_plugin").join("0.1.0");

    helpers::unpack_archive(
        &pack_plugin_path
            .join("target")
            .join("debug")
            .join("test_pack_plugin-0.1.0.tar.gz"),
        &unzipped_dir,
    );

    assert!(base_file_path.join("libtest_pack_plugin.so").exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("plugin_config.yaml").exists());
    let mig_file = base_file_path.join("migrations").join("0001_init.sql");
    let mig_file_content = fs::read_to_string(&mig_file).unwrap();
    assert!(!mig_file_content.contains("-- test"));

    // check update assets
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

    // Substitute with the current version of pike
    // TODO: #107 move it to pike_exec
    let cargo_toml = pack_plugin_path.join("Cargo.toml");
    let file = fs::File::open(&cargo_toml).unwrap();
    let reader = BufReader::new(file);

    let new_content: Vec<String> = reader
        .lines()
        .map(|line| {
            let line = line.unwrap();
            if line.starts_with("picodata-pike") {
                "picodata-pike = { path = \"../../..\" }".to_string()
            } else {
                line
            }
        })
        .collect();

    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(cargo_toml)
        .unwrap();
    for line in new_content {
        writeln!(&file, "{line}").unwrap();
    }

    exec_pike(
        vec!["plugin", "pack"],
        Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME),
        &vec![],
    );

    let unzipped_dir = pack_plugin_path.join("unzipped_release_with_changed_assets");
    let base_file_path = unzipped_dir.join("test_pack_plugin").join("0.1.0");

    helpers::unpack_archive(
        &pack_plugin_path
            .join("target")
            .join("release")
            .join("test_pack_plugin-0.1.0.tar.gz"),
        &unzipped_dir,
    );
    let mig_file = base_file_path.join("migrations").join("0001_init.sql");
    let mig_file_content = fs::read_to_string(&mig_file).unwrap();
    assert!(mig_file_content.contains("-- test"));
    let config_file = base_file_path.join("plugin_config.yaml");
    let config_content = fs::read_to_string(&config_file).unwrap();
    assert!(config_content.contains("# test"));
}

#[test]
fn test_custom_assets_with_targets() {
    let tests_dir = Path::new(TESTS_DIR);
    let plugin_path = tests_dir.join("test-plugin");

    // Cleaning up metadata from past run
    if plugin_path.exists() {
        fs::remove_dir_all(&plugin_path).unwrap();
    }

    exec_pike(vec!["plugin", "new", "test-plugin"], tests_dir, &vec![]);

    // Change build script for plugin to test custom assets
    fs::copy(
        tests_dir.join("../assets/custom_assets_with_targets_build.rs"),
        plugin_path.join("build.rs"),
    )
    .unwrap();

    // Substitute with the current version of pike
    // TODO: #107 move it to pike_exec
    let cargo_toml = plugin_path.join("Cargo.toml");
    let file = fs::File::open(&cargo_toml).unwrap();
    let reader = BufReader::new(file);

    let new_content: Vec<String> = reader
        .lines()
        .map(|line| {
            let line = line.unwrap();
            if line.starts_with("picodata-pike") {
                "picodata-pike = { path = \"../../..\" }".to_string()
            } else {
                line
            }
        })
        .collect();

    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(cargo_toml)
        .unwrap();
    for line in new_content {
        writeln!(&file, "{line}").unwrap();
    }

    // Fully test pack command for proper artifacts inside archives
    exec_pike(
        vec!["plugin", "pack"],
        TESTS_DIR,
        &vec![
            "--debug".to_string(),
            "--plugin-path".to_string(),
            "./test-plugin".to_string(),
        ],
    );

    // Check the debug archive
    let unzipped_dir = plugin_path.join("unzipped_debug");

    helpers::unpack_archive(
        &plugin_path
            .join("target")
            .join("debug")
            .join("test_plugin-0.1.0.tar.gz"),
        &unzipped_dir,
    );

    let assets_file_path = unzipped_dir.join("test_plugin").join("0.1.0");

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

    exec_pike(
        vec!["plugin", "pack"],
        TESTS_DIR,
        &vec!["--plugin-path".to_string(), "./test-plugin".to_string()],
    );

    // Check the release archive
    let unzipped_dir = plugin_path.join("unzipped_release");

    helpers::unpack_archive(
        &plugin_path
            .join("target")
            .join("release")
            .join("test_plugin-0.1.0.tar.gz"),
        &unzipped_dir,
    );

    let assets_file_path = unzipped_dir.join("test_plugin").join("0.1.0");

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
