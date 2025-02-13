mod helpers;

use helpers::{exec_pike, run_cluster, CmdArguments, PLUGIN_DIR, TESTS_DIR};
use pike::cluster::run;
use pike::cluster::Plugin;
use pike::cluster::RunParamsBuilder;
use pike::cluster::Topology;
use std::collections::BTreeMap;
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

    let mut plugins = BTreeMap::new();
    plugins.insert("test-plugin".to_string(), Plugin::default());
    let topology = Topology {
        plugins: plugins,
        ..Default::default()
    };

    dbg!(&topology);

    let mut params = RunParamsBuilder::default()
        .topology(topology)
        .data_dir(Path::new("./tmp").to_path_buf())
        .disable_plugin_install(false)
        .base_http_port(8000)
        .picodata_path(Path::new("picodata").to_path_buf())
        .base_pg_port(5432)
        .use_release(false)
        .target_dir(Path::new("./target").to_path_buf())
        .daemon(false)
        .disable_colors(false)
        .build()
        .unwrap();

    run(&mut params).unwrap();
}
