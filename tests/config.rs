mod helpers;

use helpers::{exec_pike, get_picodata_table, run_cluster, CmdArguments, PLUGIN_DIR, TESTS_DIR};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
    vec,
};

use pike::cluster::run;

use pike::cluster::Plugin;
use pike::cluster::RunParamsBuilder;
use pike::cluster::Tier;
use pike::cluster::Topology;

const TOTAL_INSTANCES: i32 = 4;

#[test]
fn test_config_apply() {
    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    assert!(exec_pike(vec!["config", "apply"], PLUGIN_DIR, &vec![])
        .unwrap()
        .success());

    let start = Instant::now();
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_plugin_config = get_picodata_table(
            Path::new(PLUGIN_DIR),
            Path::new("tmp"),
            "_pico_plugin_config",
        );
        if pico_plugin_config.contains("value") && pico_plugin_config.contains("changed") {
            return;
        }
    }

    panic!("Timeouted while trying to apply cluster config, value hasn't changed");
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_workspace_config_apply() {
    let tests_dir = Path::new(TESTS_DIR);
    let workspace_path = tests_dir.join("workspace_plugin");

    // Cleaning up metadata from past run
    if workspace_path.exists() {
        fs::remove_dir_all(&workspace_path).unwrap();
    }

    assert!(exec_pike(
        vec!["plugin", "new", "workspace_plugin"],
        tests_dir,
        &vec!["--workspace".to_string()]
    )
    .unwrap()
    .success());

    assert!(exec_pike(
        vec!["plugin", "add", "sub_plugin"],
        tests_dir,
        &vec![
            "--plugin-path".to_string(),
            "./workspace_plugin".to_string()
        ]
    )
    .unwrap()
    .success());

    let plugins = BTreeMap::from([
        ("workspace_plugin".to_string(), Plugin::default()),
        ("sub_plugin".to_string(), Plugin::default()),
    ]);

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
        .plugin_path(Path::new(&workspace_path).to_path_buf())
        .build()
        .unwrap();

    // Run cluster and check successful plugin installation
    run(&params).unwrap();

    let start = Instant::now();
    let mut is_cluster_valid = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(&workspace_path, Path::new("tmp"), "_pico_instance");
        let pico_plugin = get_picodata_table(&workspace_path, Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        // Also check that both of the plugins were enabled
        if pico_instance.matches("Online").count() == 8 && pico_plugin.matches("true").count() == 2
        {
            is_cluster_valid = true;
            break;
        }
    }
    assert!(is_cluster_valid, "Cluster didn't start successfully");

    // Test all possibilities with applying cofnig to workspace
    // 1) Apply config in all plugins

    // Change config for one plugin
    let _ = fs::remove_file(workspace_path.join("sub_plugin/plugin_config.yaml"));
    fs::copy(
        tests_dir.join("../assets/plugin_config_1.yaml"),
        workspace_path.join("sub_plugin/plugin_config.yaml"),
    )
    .unwrap();

    assert!(exec_pike(
        vec!["config", "apply"],
        TESTS_DIR,
        &vec![
            "--plugin-path".to_string(),
            "./workspace_plugin".to_string()
        ]
    )
    .unwrap()
    .success());
    is_cluster_valid = false;
    let start = Instant::now();
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_plugin_config =
            get_picodata_table(&workspace_path, Path::new("tmp"), "_pico_plugin_config");

        if pico_plugin_config.contains("value")
            && pico_plugin_config.contains("changed")
            && pico_plugin_config.contains("config1")
        {
            is_cluster_valid = true;
            break;
        }
    }

    assert!(
        is_cluster_valid,
        "Config for all plugins was not applied successfullt"
    );

    // 2) Apply config in one plugin

    let _ = fs::remove_file(workspace_path.join("sub_plugin/plugin_config.yaml"));
    fs::copy(
        tests_dir.join("../assets/plugin_config_2.yaml"),
        workspace_path.join("sub_plugin/modified_config.yaml"),
    )
    .unwrap();

    assert!(exec_pike(
        vec!["config", "apply"],
        TESTS_DIR,
        &vec![
            "--plugin-path".to_string(),
            "./workspace_plugin".to_string(),
            "--config-path".to_string(),
            "./modified_config.yaml".to_string(),
            "--plugin-name".to_string(),
            "sub_plugin".to_string()
        ]
    )
    .unwrap()
    .success());

    is_cluster_valid = false;
    let start = Instant::now();
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_plugin_config =
            get_picodata_table(&workspace_path, Path::new("tmp"), "_pico_plugin_config");

        if pico_plugin_config.contains("value")
            && pico_plugin_config.contains("changed")
            && pico_plugin_config.contains("config2")
        {
            is_cluster_valid = true;
            break;
        }
    }

    assert!(is_cluster_valid, "Failed to apply config for one plugin");

    // Test uncle Pike wise advice's
    // Forced to call Command manually instead of exec_pike to read output
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let wrong_plugin_path_cmd = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .args([
            "pike",
            "config",
            "apply",
            "--config-path",
            "./gangam_style",
            "--plugin-path",
            "./workspace_plugin",
        ])
        .current_dir(TESTS_DIR)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&wrong_plugin_path_cmd.stdout);
    assert!(
        stdout.contains("You are trying to apply config from"),
        "Failed to handle case with invalid command line arguments combination"
    );

    assert!(exec_pike(
        vec!["stop"],
        TESTS_DIR,
        &vec![
            "--data-dir".to_string(),
            "./tmp".to_string(),
            "--plugin-path".to_string(),
            "./workspace_plugin".to_string()
        ],
    )
    .unwrap()
    .success());
}
