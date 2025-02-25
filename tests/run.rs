mod helpers;

use helpers::{exec_pike, run_cluster, CmdArguments, PLUGIN_DIR};
use helpers::{get_picodata_table, TESTS_DIR};
use pike::cluster::run;
use pike::cluster::Plugin;
use pike::cluster::RunParamsBuilder;
use pike::cluster::Tier;
use pike::cluster::Topology;
use std::collections::BTreeMap;
use std::process::Command;
use std::time::Instant;
use std::{
    fs::{self},
    path::Path,
    time::Duration,
    vec,
};

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

// This code tests Pike's public interface.
// Any changes are potential BREAKING changes.
#[test]
fn test_topology_struct_run() {
    // Cleaning up metadata from past run
    if Path::new(PLUGIN_DIR).exists() {
        fs::remove_dir_all(PLUGIN_DIR).unwrap();
    }

    assert!(
        exec_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR, &vec![])
            .unwrap()
            .success()
    );

    let plugins = BTreeMap::from([("test-plugin".to_string(), Plugin::default())]);
    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);

    let topology = Topology {
        tiers,
        plugins,
        ..Default::default()
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .data_dir(Path::new("./tmp").to_path_buf())
        .disable_plugin_install(false)
        .base_http_port(8000)
        .picodata_path(Path::new("picodata").to_path_buf())
        .base_pg_port(5432)
        .use_release(false)
        .target_dir(Path::new("./target").to_path_buf())
        .daemon(true)
        .disable_colors(false)
        .plugin_path(Path::new(PLUGIN_DIR).to_path_buf())
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_plugin_config = get_picodata_table(Path::new("tmp"), "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_plugin_config.matches("Online").count() == 8 {
            cluster_started = true;
            break;
        }
    }

    assert!(exec_pike(
        vec!["stop"],
        PLUGIN_DIR,
        &vec!["--data-dir".to_string(), "./tmp".to_string()],
    )
    .unwrap()
    .success());

    assert!(cluster_started);
}

#[test]
fn test_quickstart_pipeline() {
    let quickstart_path = Path::new(TESTS_DIR).join("quickstart");
    let quickstart_plugin_dir = quickstart_path.join("test-plugin");

    // Test uncle Pike wise advice's
    // Forced to call Command manually instead of exec_pike to read output
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let wrong_plugin_path_cmd = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .args(["pike", "run"])
        .current_dir(TESTS_DIR)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&wrong_plugin_path_cmd.stdout);
    assert!(
        stdout.contains("pike outside Plugin directory"),
        "Recieved unexpected output, while trying to run pike in wrong directory, where is the fish? Output: {stdout}"
    );

    // Cleaning up metadata from past run
    if quickstart_path.exists() {
        fs::remove_dir_all(&quickstart_path).unwrap();
    }

    fs::create_dir(&quickstart_path).unwrap();
    assert!(exec_pike(
        vec!["plugin", "new", "test-plugin"],
        quickstart_path,
        &vec![]
    )
    .unwrap()
    .success());

    let plugins = BTreeMap::from([("test-plugin".to_string(), Plugin::default())]);
    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);

    let topology = Topology {
        tiers,
        plugins,
        ..Default::default()
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .data_dir(Path::new("./tmp").to_path_buf())
        .disable_plugin_install(false)
        .base_http_port(8000)
        .picodata_path(Path::new("picodata").to_path_buf())
        .base_pg_port(5432)
        .use_release(false)
        .target_dir(Path::new("./target").to_path_buf())
        .daemon(true)
        .disable_colors(false)
        .plugin_path(Path::new(&quickstart_plugin_dir).to_path_buf())
        .build()
        .unwrap();

    // Run cluster and check successful plugin installation
    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_plugin_config =
            get_picodata_table(Path::new("../quickstart/test-plugin/tmp"), "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_plugin_config.matches("Online").count() == 8 {
            cluster_started = true;
            break;
        }
    }

    assert!(exec_pike(
        vec!["stop"],
        TESTS_DIR,
        &vec![
            "--data-dir".to_string(),
            "./tmp".to_string(),
            "--plugin-path".to_string(),
            "./quickstart/test-plugin".to_string()
        ],
    )
    .unwrap()
    .success());

    assert!(cluster_started);

    // Quickly test pack command
    assert!(exec_pike(
        vec!["plugin", "pack"],
        TESTS_DIR,
        &vec![
            "--debug".to_string(),
            "--plugin-path".to_string(),
            "./quickstart/test-plugin".to_string()
        ],
    )
    .unwrap()
    .success());

    assert!(quickstart_plugin_dir
        .join("target/debug/test_plugin-0.1.0.tar.gz")
        .exists());
}
