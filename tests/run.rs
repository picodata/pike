mod helpers;

use helpers::{exec_pike, run_cluster, CmdArguments, PLUGIN_DIR};
use helpers::{get_picodata_table, TESTS_DIR};
use pike::cluster::Plugin;
use pike::cluster::RunParamsBuilder;
use pike::cluster::Service;
use pike::cluster::Tier;
use pike::cluster::Topology;
use pike::cluster::{run, MigrationContextVar};
use std::collections::BTreeMap;
use std::process::Command;
use std::time::Instant;
use std::{env, thread};
use std::{
    fs::{self},
    path::Path,
    time::Duration,
    vec,
};

use flate2::bufread::GzDecoder;
use std::{fs::File, io::BufReader};
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

    let plugins = BTreeMap::from([(
        "test-plugin".to_string(),
        Plugin {
            migration_context: vec![MigrationContextVar {
                name: "name".to_string(),
                value: "value".to_string(),
            }],
            services: BTreeMap::from([(
                "main".to_string(),
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
        let pico_instance =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_instance");
        let pico_plugin =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 && pico_plugin.contains("true") {
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
fn test_topology_struct_one_tier() {
    // Cleaning up metadata from past run
    if Path::new(PLUGIN_DIR).exists() {
        fs::remove_dir_all(PLUGIN_DIR).unwrap();
    }

    assert!(
        exec_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR, &vec![])
            .unwrap()
            .success()
    );

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 2,
            replication_factor: 2,
        },
    )]);
    let plugins = BTreeMap::from([("test-plugin".to_string(), Plugin::default())]);

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
        let pico_instance =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_instance");
        let pico_plugin =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_plugin");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 && pico_plugin.contains("true") {
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
fn test_topology_struct_run_no_plugin() {
    // Cleaning up metadata from past run
    if Path::new(PLUGIN_DIR).exists() {
        fs::remove_dir_all(PLUGIN_DIR).unwrap();
    }

    assert!(
        exec_pike(vec!["plugin", "new", "test-plugin"], TESTS_DIR, &vec![])
            .unwrap()
            .success()
    );

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
        let pico_instance =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 {
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
        let pico_plugin_config = get_picodata_table(
            &Path::new(TESTS_DIR).join("quickstart/test-plugin"),
            Path::new("tmp"),
            "_pico_instance",
        );

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

#[test]
#[allow(clippy::too_many_lines)]
fn test_workspace_pipeline() {
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

    assert!(cluster_started);

    // Fully test pack command for proper artefacts inside archives

    assert!(exec_pike(
        vec!["plugin", "pack"],
        TESTS_DIR,
        &vec![
            "--debug".to_string(),
            "--plugin-path".to_string(),
            "./workspace_plugin".to_string()
        ],
    )
    .unwrap()
    .success());

    assert!(workspace_path
        .join("target/debug/workspace_plugin-0.1.0.tar.gz")
        .exists());
    assert!(workspace_path
        .join("target/debug/sub_plugin-0.1.0.tar.gz")
        .exists());

    let build_dir = workspace_path.join("target/debug");

    // Check first plugin
    let _ = fs::create_dir(build_dir.join("tmp_workspace_plugin"));
    unpack_archive(
        &build_dir.join("workspace_plugin-0.1.0.tar.gz"),
        &build_dir.join("tmp_workspace_plugin"),
    );

    assert!(build_dir
        .join("tmp_workspace_plugin/libworkspace_plugin.so")
        .exists());
    assert!(build_dir
        .join("tmp_workspace_plugin/manifest.yaml")
        .exists());
    assert!(build_dir.join("tmp_workspace_plugin/migrations").is_dir());

    // Check second plugin with custom assets
    let _ = fs::create_dir(build_dir.join("tmp_sub_plugin"));
    unpack_archive(
        &build_dir.join("sub_plugin-0.1.0.tar.gz"),
        &build_dir.join("tmp_sub_plugin"),
    );

    assert!(build_dir.join("tmp_sub_plugin/libsub_plugin.so").exists());
    assert!(build_dir.join("tmp_sub_plugin/manifest.yaml").exists());
    assert!(build_dir.join("tmp_sub_plugin/migrations").is_dir());
    assert!(build_dir.join("tmp_sub_plugin/topology.toml").exists());
}

fn unpack_archive(path: &Path, unpack_to: &Path) {
    let tar_archive = File::open(path).unwrap();
    let buf_reader = BufReader::new(tar_archive);
    let decompressor = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(decompressor);

    archive.unpack(unpack_to).unwrap();
}

#[test]
fn test_run_without_plugin_directory() {
    let run_dir = Path::new(TESTS_DIR);
    let plugin_dir = Path::new("test_run_without_plugin_directory");
    let data_dir = plugin_dir.join("tmp");

    // Cleaning up metadata from past run
    let _ = fs::remove_dir_all(run_dir.join(plugin_dir));

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
        .data_dir(run_dir.join(&data_dir))
        .daemon(true)
        .build()
        .unwrap();

    run(&params).unwrap();

    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance = get_picodata_table(run_dir, &data_dir, "_pico_instance");

        // Compare with 8, because table gives current state and target state
        // both of them should be online
        if pico_instance.matches("Online").count() == 8 {
            cluster_started = true;
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }

    assert!(exec_pike(
        vec!["stop"],
        env::current_dir().unwrap(),
        &vec![
            "--data-dir".to_string(),
            run_dir.join(&data_dir).to_str().unwrap().to_string()
        ],
    )
    .unwrap()
    .success());

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
        // +-------------+---------+----------+---------+-----------------------+------------------------------+
        // | name        | enabled | services | version | description           | migration_list               |
        // +===================================================================================================+
        // | test-plugin | true    | ["main"] | 0.1.0   | A plugin for picodata | ["migrations/0001_init.sql"] |
        // +-------------+---------+----------+---------+-----------------------+------------------------------+
        let pico_plugin =
            get_picodata_table(Path::new(PLUGIN_DIR), Path::new("tmp"), "_pico_plugin");
        if !pico_plugin.contains("true") {
            dbg!(pico_plugin);
            continue;
        }

        // example value:
        // +-------------+------+---------+---------------------+-----------------+
        // | plugin_name | name | version | tiers               | description     |
        // +======================================================================+
        // | test-plugin | main | 0.1.0   | ["second", "third"] | default service |
        // +-------------+------+---------+---------------------+-----------------+
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
