mod helpers;

use flate2::bufread::GzDecoder;
use helpers::{
    build_plugin, check_plugin_version_artefacts, cleanup_dir, get_picodata_table, run_cluster,
    run_pike, wait_for_proc, CmdArguments, PACK_PLUGIN_NAME, PLUGIN_DIR, TESTS_DIR,
};
use std::{
    fs::{self, File},
    io::BufReader,
    path::Path,
    time::Duration,
    vec,
};
use tar::Archive;

const TOTAL_INSTANCES: i32 = 4;

#[test]
fn test_cluster_setup_debug() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();
}

#[test]
fn test_cluster_setup_release() {
    let run_params = CmdArguments {
        run_args: ["--release", "--data-dir", "new_data_dir"]
            .iter()
            .map(|&s| s.into())
            .collect(),
        stop_args: ["--data-dir", "new_data_dir"]
            .iter()
            .map(|&s| s.into())
            .collect(),
        ..Default::default()
    };

    let _cluster_handle =
        run_cluster(Duration::from_secs(120), TOTAL_INSTANCES, run_params).unwrap();
}

#[test]
fn test_config_apply() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    let mut plugin_creation_proc = run_pike(vec!["config", "apply"], PLUGIN_DIR, &vec![]).unwrap();

    wait_for_proc(&mut plugin_creation_proc, Duration::from_secs(10));

    let pico_plugin_config = get_picodata_table(Path::new("tmp"), "_pico_plugin_config");

    assert!(pico_plugin_config.contains("value") && pico_plugin_config.contains("changed"));
}

// Using as much command line arguments in this test as we can
#[test]
fn test_cluster_daemon_and_arguments() {
    let run_params = CmdArguments {
        run_args: [
            "-d",
            "--topology",
            "../../assets/topology.toml",
            "--base-http-port",
            "8001",
            "--base-pg-port",
            "5430",
            "--target-dir",
            "tmp_target",
        ]
        .iter()
        .map(|&s| s.into())
        .collect(),
        build_args: ["--target-dir", "tmp_target"]
            .iter()
            .map(|&s| s.into())
            .collect(),
        plugin_args: vec!["--workspace".to_string()],
        ..Default::default()
    };

    let _cluster_handle =
        run_cluster(Duration::from_secs(120), TOTAL_INSTANCES, run_params).unwrap();

    // Validate each instances's PID
    for entry in fs::read_dir(Path::new(PLUGIN_DIR).join("tmp").join("cluster")).unwrap() {
        let entry = entry.unwrap();
        let pid_path = entry.path().join("pid");

        assert!(pid_path.exists());

        if let Ok(content) = fs::read_to_string(&pid_path) {
            assert!(content.trim().parse::<u32>().is_ok());
        }
    }
}

#[test]
fn test_cargo_build() {
    // Cleaning up metadata from past run
    if Path::new(PLUGIN_DIR).exists() {
        fs::remove_dir_all(PLUGIN_DIR).unwrap();
    }

    let mut plugin_creation_proc =
        run_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR, &vec![]).unwrap();

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

#[test]
fn test_cargo_pack() {
    cleanup_dir(&Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME));

    let mut plugin_creation_proc =
        run_pike(vec!["plugin", "new", PACK_PLUGIN_NAME], TESTS_DIR, &vec![]).unwrap();

    wait_for_proc(&mut plugin_creation_proc, Duration::from_secs(10));

    let mut plugin_pack_proc = run_pike(
        vec!["plugin", "pack"],
        Path::new(TESTS_DIR).join(PACK_PLUGIN_NAME),
        &vec!["--target-dir".to_string(), "tmp_target".to_string()],
    )
    .unwrap();

    wait_for_proc(&mut plugin_pack_proc, Duration::from_secs(120));

    // Hail for archive handling in Rust
    let plugin_path = Path::new(TESTS_DIR)
        .join(PACK_PLUGIN_NAME)
        .join("tmp_target");
    let tar_archive = File::open(plugin_path.join("test_pack_plugin-0.1.0.tar.gz")).unwrap();
    let buf_reader = BufReader::new(tar_archive);
    let decompressor = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(decompressor);

    archive.unpack(&plugin_path).unwrap();

    assert!(plugin_path.join("libtest_pack_plugin.so").exists());
    assert!(plugin_path.join("manifest.yaml").exists());
    assert!(plugin_path.join("migrations").is_dir());
}
