mod helpers;

use helpers::{
    build_plugin, cleanup_dir, exec_pike, exec_pike_in, get_picodata_table, init_plugin,
    init_plugin_with_args, init_plugin_workspace, run_cluster, wait_cluster_start_completed,
};
use helpers::{CmdArguments, TestPluginInitParams, LIB_EXT, PLUGIN_DIR, PLUGIN_NAME, TESTS_DIR};
use pike::cluster::{run, MigrationContextVar, Plugin, RunParamsBuilder, Service, Tier, Topology};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::{
    fs::{self},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use crate::helpers::is_instance_running;

const TOTAL_INSTANCES: i32 = 4;

/// Find archive with new naming format: <name>_<version>-<osid>_<variant>.tar.gz
/// Ensures exactly one archive matches.
fn find_os_suffixed_archive(dir: &Path, name: &str, version: &str) -> PathBuf {
    let prefix = format!("{name}_{version}-");
    let mut matches = vec![];
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("Cannot read build dir {dir:?} while searching archives"));
    for entry in entries {
        let entry = entry.unwrap();
        let fname = entry.file_name();
        let fname = fname.to_string_lossy();
        if fname.starts_with(&prefix) && fname.ends_with(".tar.gz") {
            matches.push(entry.path());
        }
    }
    assert!(
        !matches.is_empty(),
        "No archive found in {dir:?} with prefix {prefix}"
    );
    assert_eq!(
        matches.len(),
        1,
        "Expected exactly one archive for {name} {version}, found {}: {:?}",
        matches.len(),
        matches
    );
    matches.remove(0)
}

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
    let plugin_path = Path::new(PLUGIN_DIR);

    init_plugin(PLUGIN_NAME);

    let plugins = BTreeMap::from([(
        PLUGIN_NAME.to_string(),
        Plugin {
            migration_context: vec![MigrationContextVar {
                name: "name".to_string(),
                value: "value".to_string(),
            }],
            services: BTreeMap::from([(
                "example_service".to_string(),
                Service {
                    tiers: vec!["default".to_string()],
                },
            )]),
            ..Default::default()
        },
    )]);

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
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_instance");
        let pico_plugin = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 && pico_plugin.contains("true") {
            cluster_started = true;
            break;
        }
    }

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn test_multiple_run_attempt() {
    let plugin_path = Path::new(PLUGIN_DIR);

    init_plugin(PLUGIN_NAME);

    let plugins = BTreeMap::from([(
        PLUGIN_NAME.to_string(),
        Plugin {
            migration_context: vec![MigrationContextVar {
                name: "name".to_string(),
                value: "value".to_string(),
            }],
            services: BTreeMap::from([(
                "example_service".to_string(),
                Service {
                    tiers: vec!["default".to_string()],
                },
            )]),
            ..Default::default()
        },
    )]);

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
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_instance");
        let pico_plugin = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 && pico_plugin.contains("true") {
            cluster_started = true;
            break;
        }
    }

    // Ensure that we stop picodata cluster before panicing
    let res = run(&params);
    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(
        res.is_err(),
        "Expected to fail while trying to run multiple clusters"
    );

    let err_str = res.unwrap_err().to_string();
    assert!(
        err_str.contains("cluster has already started, can connect via"),
        "Wrong error message while trying to run multiple clusters: {err_str}"
    );

    assert!(cluster_started);
}

#[test]
fn test_cluster_failure() {
    let plugin_path = Path::new(PLUGIN_DIR);

    init_plugin(PLUGIN_NAME);

    // Write trash inside migraions file
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(plugin_path.join("migrations/0001_init.sql"))
        .unwrap();

    writeln!(file, "Scooby do be do, where are you?").unwrap();
    writeln!(file, "We got some work to do nooow!").unwrap();

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);
    let plugins = BTreeMap::from([(PLUGIN_NAME.to_string(), Plugin::default())]);

    let topology = Topology {
        tiers,
        plugins,
        ..Default::default()
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    let cluster_status = run(&params);
    assert!(cluster_status.is_err(), "Expected migration error");

    let err = cluster_status.unwrap_err();
    assert!(
        err.to_string().contains("MIGRATE"),
        "expected migration error"
    );
}

#[test]
fn test_topology_struct_one_tier() {
    let plugin_path = Path::new(PLUGIN_DIR);

    init_plugin(PLUGIN_NAME);

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);
    let plugins = BTreeMap::from([(PLUGIN_NAME.to_string(), Plugin::default())]);

    let topology = Topology {
        tiers,
        plugins,
        ..Default::default()
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_instance");
        let pico_plugin = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 && pico_plugin.contains("true") {
            cluster_started = true;
            break;
        }
    }

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn test_topology_struct_run_no_plugin() {
    let plugin_path = Path::new(PLUGIN_DIR);

    init_plugin(PLUGIN_NAME);

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);

    let topology = Topology {
        tiers,
        ..Default::default()
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(plugin_path, Path::new("tmp"), "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 {
            cluster_started = true;
            break;
        }
    }

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn test_picodata_instance_interaction() {
    let plugin_path = Path::new(PLUGIN_DIR);

    init_plugin(PLUGIN_NAME);

    let plugins = BTreeMap::from([(
        PLUGIN_NAME.to_string(),
        Plugin {
            migration_context: vec![MigrationContextVar {
                name: "name".to_string(),
                value: "value".to_string(),
            }],
            services: BTreeMap::from([(
                "example_service".to_string(),
                Service {
                    tiers: vec!["default".to_string()],
                },
            )]),
            ..Default::default()
        },
    )]);

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
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    let pico_instances = run(&params).unwrap();
    let properties = pico_instances.first().unwrap().properties();

    assert_eq!(properties.bin_port, &3001);
    assert_eq!(properties.http_port, &8001);
    assert_eq!(properties.pg_port, &5433);
    assert_eq!(properties.instance_id, &1);
    assert_eq!(properties.tier, "default");
    assert_eq!(properties.instance_name, "default_1_1");
    assert_eq!(
        properties.data_dir.to_str().unwrap(),
        "./tests/tmp/test-plugin/./tmp/cluster/i1"
    );

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);
}

#[test]
fn test_quickstart_pipeline() {
    let quickstart_path = Path::new(TESTS_DIR).join("quickstart");

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
        "Received unexpected output, while trying to run pike in wrong directory, where is the fish? Output: {stdout}"
    );

    init_plugin("quickstart");

    let plugins = BTreeMap::from([("quickstart".to_string(), Plugin::default())]);
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
        .daemon(true)
        .plugin_path(quickstart_path.clone())
        .build()
        .unwrap();

    // Run cluster and check successful plugin installation
    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_plugin_config =
            get_picodata_table(&quickstart_path, Path::new("tmp"), "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_plugin_config.matches("Online").count() == 8 {
            cluster_started = true;
            break;
        }
    }

    exec_pike(["stop", "--plugin-path", "quickstart"]);
    assert!(cluster_started);

    // Quickly test pack command
    exec_pike(["plugin", "pack", "--debug", "--plugin-path", "quickstart"]);

    let build_dir = quickstart_path.join("target/debug");
    let quickstart_archive = find_os_suffixed_archive(&build_dir, "quickstart", "0.1.0");
    assert!(
        quickstart_archive.exists(),
        "Expected quickstart archive at {quickstart_archive:?}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_workspace_pipeline() {
    let tests_dir = Path::new(TESTS_DIR);
    let workspace_path = tests_dir.join("workspace_plugin");

    init_plugin_workspace("workspace_plugin");

    exec_pike([
        "plugin",
        "add",
        "sub_plugin",
        "--plugin-path",
        "workspace_plugin",
    ]);

    let plugins = BTreeMap::from([
        ("workspace_plugin".to_string(), Plugin::default()),
        ("sub_plugin".to_string(), Plugin::default()),
    ]);

    // Change build script for sub plugin to test custom assets
    fs::copy(
        tests_dir.join("../assets/custom_assets_build.rs"),
        workspace_path.join("sub_plugin/build.rs"),
    )
    .unwrap();

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
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(&workspace_path, Path::new("tmp"), "_pico_instance");
        let pico_plugin = get_picodata_table(&workspace_path, Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        // Also check that both of the plugins were enabled
        if pico_instance.matches("Online").count() == 8 && pico_plugin.matches("true").count() == 2
        {
            cluster_started = true;
            break;
        }
    }

    exec_pike(["stop", "--plugin-path", "workspace_plugin"]);
    assert!(cluster_started);

    // Fully test pack command for proper artefacts inside archives
    exec_pike([
        "plugin",
        "pack",
        "--debug",
        "--plugin-path",
        "workspace_plugin",
    ]);

    let build_dir = workspace_path.join("target/debug");
    let workspace_archive = find_os_suffixed_archive(&build_dir, "workspace_plugin", "0.1.0");
    let sub_archive = find_os_suffixed_archive(&build_dir, "sub_plugin", "0.1.0");

    // Unpack first plugin
    let _ = fs::create_dir(build_dir.join("tmp_workspace_plugin"));
    helpers::unpack_archive(&workspace_archive, &build_dir.join("tmp_workspace_plugin"));

    let base_file_path = build_dir
        .join("tmp_workspace_plugin")
        .join("workspace_plugin")
        .join("0.1.0");
    assert!(base_file_path
        .join(format!("libworkspace_plugin.{LIB_EXT}"))
        .exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("migrations").is_dir());

    // Unpack second plugin
    let _ = fs::create_dir(build_dir.join("tmp_sub_plugin"));
    helpers::unpack_archive(&sub_archive, &build_dir.join("tmp_sub_plugin"));

    let base_file_path = build_dir
        .join("tmp_sub_plugin")
        .join("sub_plugin")
        .join("0.1.0");
    assert!(base_file_path
        .join(format!("libsub_plugin.{LIB_EXT}"))
        .exists());
    assert!(base_file_path.join("manifest.yaml").exists());
    assert!(base_file_path.join("migrations").is_dir());
    assert!(base_file_path.join("plugin_config.yaml").exists());
}

#[test]
fn test_run_without_plugin_directory() {
    let plugin_dir = Path::new(TESTS_DIR).join("test_run_without_plugin_directory");

    cleanup_dir(&plugin_dir);

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);

    let topology = Topology {
        tiers,
        ..Default::default()
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .plugin_path(plugin_dir.clone())
        .daemon(true)
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(&plugin_dir, Path::new("tmp"), "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 {
            cluster_started = true;
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }

    exec_pike(["stop", "--plugin-path", "test_run_without_plugin_directory"]);

    assert!(cluster_started);
}

#[test]
fn test_run_with_several_tiers() {
    let run_params = CmdArguments {
        run_args: vec![
            "-d".into(),
            "--topology".into(),
            "../../assets/topology_several_tiers.toml".into(),
        ],
        ..Default::default()
    };

    let _cluster_handle = run_cluster(Duration::from_secs(120), 6, run_params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        thread::sleep(Duration::from_secs(1));

        // example value:
        // +-------------+--------------------------------------+---------+-----------------+--------------------------------------+---------------+---------------+----------------+---------+--------------------+
        // | name        | uuid                                 | raft_id | replicaset_name | replicaset_uuid                      | current_state | target_state  | failure_domain | tier    | picodata_version   |
        // +=======================================================================================================================================================================================================+
        // | default_1_1 | 4d607252-4603-42bf-88fa-c4b1bb4fab23 | 1       | default_1       | 25d1dfd1-bbb4-4fd0-880f-77b7512b07b6 | ["Online", 1] | ["Online", 1] | {}             | default | 25.1.1-0-g38230552 |
        // |-------------+--------------------------------------+---------+-----------------+--------------------------------------+---------------+---------------+----------------+---------+--------------------|
        // | default_1_2 | ef6ccfee-c855-479b-a15a-a050a6493d08 | 2       | default_1       | 25d1dfd1-bbb4-4fd0-880f-77b7512b07b6 | ["Online", 1] | ["Online", 1] | {}             | default | 25.1.1-0-g38230552 |
        // |-------------+--------------------------------------+---------+-----------------+--------------------------------------+---------------+---------------+----------------+---------+--------------------|
        let pico_instance =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_instance");

        // Tier default == 1 replicaset and replication_factor is 3 => "default" must be met 9 times
        if pico_instance.matches("default").count() != 9 {
            dbg!(pico_instance);
            continue;
        }
        // Tier second == 1 replicaset and replication_factor is 1 => "second" must be met 3 times
        if pico_instance.matches("second").count() != 3 {
            dbg!(pico_instance);
            continue;
        }
        // Tier third == 1 replicaset and replication_factor is 2 => "third" must be met 6 times
        if pico_instance.matches("third").count() != 6 {
            dbg!(pico_instance);
            continue;
        }
        // Total instances is 6 => "Online" must be meet 12 times
        if pico_instance.matches("Online").count() != 12 {
            dbg!(pico_instance);
            continue;
        }

        // example value:
        // +-------------+---------+---------------------+---------+-----------------------+------------------------------+
        // | name        | enabled | services            | version | description           | migration_list               |
        // +==============================================================================================================+
        // | test-plugin | true    | ["example_service"] | 0.1.0   | A plugin for picodata | ["migrations/0001_init.sql"] |
        // +-------------+---------+---------------------+---------+-----------------------+------------------------------+
        let pico_plugin =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_plugin");
        if !pico_plugin.contains("true") {
            dbg!(pico_plugin);
            continue;
        }

        // example value:
        // +-------------+-----------------+---------+---------------------+-----------------+
        // | plugin_name | name            | version | tiers               | description     |
        // +=================================================================================+
        // | test-plugin | example_service | 0.1.0   | ["second", "third"] | default service |
        // +-------------+-----------------+---------+---------------------+-----------------+
        let pico_service =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_service");
        if !(pico_service.contains("second") && pico_service.contains("third")) {
            dbg!(pico_service);
            continue;
        }

        cluster_started = true;
    }

    assert!(cluster_started);
}

/// Create simple pike run parameters using provided plugins.
fn make_ext_run_params(plugin_path: &Path, plugins: BTreeMap<String, Plugin>) -> RunParamsBuilder {
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

    let mut builder = RunParamsBuilder::default();
    builder
        .topology(topology)
        .daemon(true)
        .plugin_path(plugin_path.into());
    builder
}

#[test]
fn run_with_external_plugin_directory() {
    let plugin_path = Path::new(PLUGIN_DIR);
    init_plugin(PLUGIN_NAME);
    build_plugin(&helpers::BuildType::Debug, "0.1.0", plugin_path);

    let ext_plugin_path = PathBuf::from("./tests/tmp_ext/external-plugin-1");
    init_plugin_with_args(TestPluginInitParams::<String> {
        name: "external-plugin-1".to_string(),
        plugin_path: ext_plugin_path.clone(),
        shared_target_path: "./tests/tmp_ext/ext_shared_target".into(),
        working_dir: "./tests/tmp_ext".into(),
        ..Default::default()
    });
    build_plugin(&helpers::BuildType::Debug, "0.1.0", &ext_plugin_path);

    let our_plugin_path = Path::new("./tests/tmp/test-plugin");
    let ext_plugin_path =
        Path::new("./tests/tmp_ext/external-plugin-1/target/debug/external-plugin-1");

    let our_plugin = Plugin::default();
    let external_plugin = Plugin {
        path: Some(ext_plugin_path.into()),
        ..Default::default()
    };
    let plugins = BTreeMap::from([
        (PLUGIN_NAME.to_string(), our_plugin),
        ("external-plugin-1".to_string(), external_plugin),
    ]);
    let params = make_ext_run_params(our_plugin_path, plugins)
        .build()
        .unwrap();

    run(&params).unwrap();

    let cluster_started = wait_cluster_start_completed(our_plugin_path, |state| {
        assert_eq!(state.pico_instance.matches("Online").count(), 8);
        assert_eq!(state.pico_plugin.matches("true").count(), 2);
        true
    });

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn run_with_external_plugin_archive() {
    let plugin_path = Path::new(PLUGIN_DIR);
    init_plugin(PLUGIN_NAME);
    build_plugin(&helpers::BuildType::Debug, "0.1.0", plugin_path);

    // setup and pack external plugin
    let ext_plugin_root = PathBuf::from("./tests/tmp_ext/external-plugin-1");
    init_plugin_with_args(TestPluginInitParams::<String> {
        name: "external-plugin-1".to_string(),
        plugin_path: ext_plugin_root.join(""),
        shared_target_path: "./tests/tmp_ext/ext_shared_target".into(),
        working_dir: "./tests/tmp_ext".into(),
        ..Default::default()
    });
    build_plugin(&helpers::BuildType::Release, "0.1.0", &ext_plugin_root);
    let pack_args = ["plugin", "pack", "--plugin-path", "./external-plugin-1"];
    exec_pike_in(pack_args, "./tests/tmp_ext");

    // Find new-format archive
    let ext_release_dir = Path::new("./tests/tmp_ext/external-plugin-1/target/release");
    let ext_plugin_archive =
        find_os_suffixed_archive(ext_release_dir, "external-plugin-1", "0.1.0");

    let plugins = BTreeMap::from([
        (PLUGIN_NAME.to_string(), Plugin::default()),
        (
            "external-plugin-1".to_string(),
            Plugin {
                path: Some(ext_plugin_archive),
                ..Default::default()
            },
        ),
    ]);
    let params = make_ext_run_params(Path::new("./tests/tmp/test-plugin"), plugins)
        .build()
        .unwrap();

    run(&params).unwrap();

    let cluster_started =
        wait_cluster_start_completed(Path::new("./tests/tmp/test-plugin"), |state| {
            assert_eq!(state.pico_instance.matches("Online").count(), 8);
            assert_eq!(state.pico_plugin.matches("true").count(), 2);
            true
        });

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn run_with_external_plugin_project() {
    let plugin_path = Path::new(PLUGIN_DIR);
    init_plugin(PLUGIN_NAME);
    build_plugin(&helpers::BuildType::Debug, "0.1.0", plugin_path);

    // init external plugin and do not build it - we'll check that run calls "cargo build"
    let ext_plugin_path = PathBuf::from("./tests/tmp_ext/external-plugin-1");
    init_plugin_with_args(TestPluginInitParams::<String> {
        name: "external-plugin-1".to_string(),
        plugin_path: ext_plugin_path,
        shared_target_path: "./tests/tmp_ext/ext_shared_target".into(),
        working_dir: "./tests/tmp_ext".into(),
        ..Default::default()
    });

    let our_plugin_path = Path::new("./tests/tmp/test-plugin");
    let ext_plugin_path = Path::new("./tests/tmp_ext/external-plugin-1");

    let plugins = BTreeMap::from([
        (PLUGIN_NAME.to_string(), Plugin::default()),
        (
            "external-plugin-1".to_string(),
            Plugin {
                path: Some(ext_plugin_path.into()),
                ..Default::default()
            },
        ),
    ]);
    let params = make_ext_run_params(our_plugin_path, plugins)
        .build()
        .unwrap();

    run(&params).unwrap();

    let cluster_started = wait_cluster_start_completed(our_plugin_path, |state| {
        assert_eq!(state.pico_instance.matches("Online").count(), 8);
        assert_eq!(state.pico_plugin.matches("true").count(), 2);
        true
    });

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn run_with_external_plugin_workspace() {
    let plugin_path = Path::new(PLUGIN_DIR);
    init_plugin(PLUGIN_NAME);
    build_plugin(&helpers::BuildType::Debug, "0.1.0", plugin_path);

    let ext_workspace_path = Path::new("./tests/tmp_ext/ext-workspace-plugin");
    init_plugin_with_args(TestPluginInitParams {
        name: "ext-workspace-plugin".to_string(),
        plugin_path: ext_workspace_path.to_owned(),
        init_args: vec!["--workspace"],
        shared_target_path: "./tests/tmp_ext/ext_shared_target".into(),
        working_dir: "./tests/tmp_ext".into(),
    });
    exec_pike_in(["plugin", "add", "ext-sub-plugin"], ext_workspace_path);

    let plugins = BTreeMap::from([
        (PLUGIN_NAME.to_string(), Plugin::default()),
        (
            "ext-workspace-plugin".to_string(),
            Plugin {
                path: Some(ext_workspace_path.into()),
                ..Default::default()
            },
        ),
        (
            "ext-sub-plugin".to_string(),
            Plugin {
                path: Some(ext_workspace_path.into()),
                ..Default::default()
            },
        ),
    ]);
    let params = make_ext_run_params(plugin_path, plugins).build().unwrap();

    run(&params).unwrap();

    let cluster_started = wait_cluster_start_completed(plugin_path, |state| {
        assert_eq!(state.pico_instance.matches("Online").count(), 8);
        assert_eq!(state.pico_plugin.matches("true").count(), 3);
        true
    });

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn run_specific_instance() {
    let plugin_path = Path::new(PLUGIN_DIR);
    init_plugin(PLUGIN_NAME);
    let target_instance = "i2";

    let _cluster_handle = run_cluster(
        Duration::from_secs(120),
        TOTAL_INSTANCES,
        CmdArguments::default(),
    )
    .unwrap();

    // Stop single instance in the cluster.
    exec_pike([
        "stop",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        target_instance,
    ]);

    let data_dir = plugin_path.join("tmp").join("cluster");
    let instance_dir = data_dir.join(target_instance);

    // Wait while stopping instance is not killed.
    let start = Instant::now();
    let timeout = Duration::from_secs(60);

    while is_instance_running(&instance_dir) {
        thread::sleep(Duration::from_secs(1));

        assert!(
            Instant::now().duration_since(start) < timeout,
            "Timeout has reached. Instance was not stopped."
        );
    }

    // Check that all other instances were not killed.
    for entry in fs::read_dir(&data_dir).unwrap() {
        let instance_dir = entry.unwrap().path();

        // Skip symlinks.
        if fs::symlink_metadata(&instance_dir).unwrap().is_symlink() {
            continue;
        }

        // Skip target instance (it's stopped).
        if instance_dir.file_name().unwrap() == target_instance {
            continue;
        }

        // All other instances should be running.
        assert!(
            is_instance_running(&instance_dir),
            "No any instance should be killed except passed in --instance-name"
        );
    }

    // Start single instance
    exec_pike([
        "run",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        target_instance,
        "--daemon",
    ]);

    let cluster_started = wait_cluster_start_completed(plugin_path, |state| {
        assert_eq!(state.pico_instance.matches("Online").count(), 8);
        true
    });

    assert!(cluster_started);

    // Test skipping the start single instance
    exec_pike([
        "run",
        "--plugin-path",
        PLUGIN_NAME,
        "--instance-name",
        target_instance,
        "--daemon",
    ]);

    let cluster_started = wait_cluster_start_completed(plugin_path, |state| {
        assert_eq!(state.pico_instance.matches("Online").count(), 8);
        true
    });

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);

    assert!(cluster_started);
}

#[test]
fn run_with_env_variables() {
    let plugin_path = Path::new(PLUGIN_DIR);
    init_plugin(PLUGIN_NAME);

    let plugins = BTreeMap::from([(
        PLUGIN_NAME.to_string(),
        Plugin {
            migration_context: vec![MigrationContextVar {
                name: "name".to_string(),
                value: "value".to_string(),
            }],
            services: BTreeMap::from([(
                "example_service".to_string(),
                Service {
                    tiers: vec!["default".to_string()],
                },
            )]),
            ..Default::default()
        },
    )]);

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);

    let enviroment = BTreeMap::from_iter([
        (
            String::from("PICODATA_HTTP_LISTEN"),
            String::from("0.0.0.0:{{ instance_id | plus: 18000 }}"),
        ),
        (
            String::from("PICODATA_PG_LISTEN"),
            String::from("127.0.0.1:{{ instance_id | plus: 5400 }}"),
        ),
        (
            String::from("PICODATA_IPROTO_LISTEN"),
            String::from("127.0.0.1:{{ instance_id | plus: 3300 }}"),
        ),
    ]);

    let topology = Topology {
        enviroment,
        plugins,
        tiers,
        pre_install_sql: vec![],
    };
    let params = RunParamsBuilder::default()
        .topology(topology)
        .daemon(true)
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();

    let pico_instances = run(&params).unwrap();
    let properties = pico_instances.first().unwrap().properties();

    assert_eq!(properties.bin_port, &3301);
    assert_eq!(properties.http_port, &18001);
    assert_eq!(properties.pg_port, &5401);
    assert_eq!(properties.instance_id, &1);
    assert_eq!(properties.tier, "default");
    assert_eq!(properties.instance_name, "default_1_1");
    assert_eq!(
        properties.data_dir.to_str().unwrap(),
        "./tests/tmp/test-plugin/./tmp/cluster/i1"
    );

    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);
}
