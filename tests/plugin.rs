mod helpers;

use flate2::bufread::GzDecoder;
use helpers::{cleanup_dir, exec_pike, PLUGIN_DIR, TESTS_DIR};
use std::{
    fs::{self, File},
    io::BufReader,
    path::Path,
    vec,
};
use tar::Archive;

pub const PACK_PLUGIN_NAME: &str = "test-pack-plugin";

#[test]
fn test_cargo_pack() {
    cleanup_dir(&Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME));

    assert!(
        exec_pike(vec!["plugin", "new", PACK_PLUGIN_NAME], TESTS_DIR, &vec![])
            .unwrap()
            .success()
    );

    assert!(exec_pike(
        vec!["plugin", "pack"],
        Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME),
        &vec!["--target-dir".to_string(), "tmp_target".to_string()],
    )
    .unwrap()
    .success());

    // Hail for archive handling in Rust
    let plugin_path = Path::new(TESTS_DIR)
        .join(PACK_PLUGIN_NAME)
        .join("tmp_target");
    let tar_archive =
        File::open(plugin_path.join("release/test_pack_plugin-0.1.0.tar.gz")).unwrap();
    let buf_reader = BufReader::new(tar_archive);
    let decompressor = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(decompressor);

    archive.unpack(&plugin_path).unwrap();

    assert!(plugin_path.join("libtest_pack_plugin.so").exists());
    assert!(plugin_path.join("manifest.yaml").exists());
    assert!(plugin_path.join("migrations").is_dir());
}

#[test]
fn test_cargo_plugin_new() {
    cleanup_dir(&Path::new(PLUGIN_DIR).to_path_buf());

    assert!(
        exec_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR, &vec![])
            .unwrap()
            .success()
    );

    assert!(Path::new(PLUGIN_DIR).join("picodata.yaml").exists());
    assert!(Path::new(PLUGIN_DIR).join(".git").exists());
    assert!(Path::new(PLUGIN_DIR).join("topology.toml").exists());
    assert!(Path::new(PLUGIN_DIR)
        .join("manifest.yaml.template")
        .exists());

    cleanup_dir(&Path::new(PLUGIN_DIR).to_path_buf());

    assert!(exec_pike(
        vec!["plugin", "new", "test-plugin", "--without-git"],
        TESTS_DIR,
        &vec![],
    )
    .unwrap()
    .success());

    assert!(!Path::new(PLUGIN_DIR).join(".git").exists());

    cleanup_dir(&Path::new(PLUGIN_DIR).to_path_buf());

    // Test creating plugin as workspace
    exec_pike(
        vec!["plugin", "new", "test-plugin", "--workspace"],
        TESTS_DIR,
        &vec![],
    )
    .unwrap();

    assert!(Path::new(PLUGIN_DIR).join("test-plugin").exists());

    let contents = fs::read_to_string(Path::new(PLUGIN_DIR).join("Cargo.toml")).unwrap();
    assert!(contents.contains("[workspace]"));
}
