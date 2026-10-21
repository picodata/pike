mod helpers;

use helpers::{
    build_plugin, exec_pike, get_picodata_table, init_plugin, init_plugin_with_args,
    wait_cluster_start_completed, TestPluginInitParams, PLUGIN_DIR, PLUGIN_NAME,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use pike::cluster::{run, Plugin, RunParamsBuilder, Service, Tier, Topology};
use pike::config::PluginConfigMap;

const SERVICE: &str = "example_service";

/// Makes `on_start` fail unless the service sees `expected` as the value of its config,
/// so the plugin gets enabled only if the config was applied before `ENABLE`.
fn require_config_value_in_on_start(plugin_dir: &Path, expected: &str) {
    let service_rs = plugin_dir.join("src/service.rs");
    let src = fs::read_to_string(&service_rs).unwrap();
    let patched = src.replace(
        "_ = config;\n",
        &format!(
            "if config.value.as_deref() != Some({expected:?}) {{\n\
             return Err(format!(\"unexpected config: {{:?}}\", config.value).into());\n\
             }}\n"
        ),
    );
    assert_ne!(src, patched, "failed to patch on_start");
    fs::write(service_rs, patched).unwrap();
}

fn config_map(value: &str) -> PluginConfigMap {
    HashMap::from([(
        SERVICE.to_string(),
        HashMap::from([("value".to_string(), serde_norway::to_value(value).unwrap())]),
    )])
}

fn topology(plugins: BTreeMap<String, Plugin>) -> Topology {
    Topology {
        tiers: BTreeMap::from([(
            "default".to_string(),
            Tier {
                replicasets: 1,
                replication_factor: 1,
            },
        )]),
        plugins,
        ..Default::default()
    }
}

fn plugin_with_service() -> Plugin {
    Plugin {
        services: BTreeMap::from([(
            SERVICE.to_string(),
            Service {
                tiers: vec!["default".to_string()],
            },
        )]),
        ..Default::default()
    }
}

fn params_builder(topology: Topology) -> RunParamsBuilder {
    let mut builder = RunParamsBuilder::default();
    builder
        .topology(topology)
        .daemon(true)
        .plugin_path(PathBuf::from(PLUGIN_DIR));
    builder
}

/// Waits until the plugin is enabled (it is, only if `on_start` succeeded).
fn assert_plugin_enabled(plugin_path: &str, plugins: usize) {
    let started = wait_cluster_start_completed(plugin_path, |state| {
        assert_eq!(state.pico_plugin.matches("true").count(), plugins);
        true
    });
    assert!(started, "plugin was not enabled, on_start probably failed");
}

fn stop() {
    exec_pike(["stop", "--plugin-path", PLUGIN_NAME]);
}

#[test]
fn cli_run_applies_topology_config() {
    init_plugin(PLUGIN_NAME);
    require_config_value_in_on_start(Path::new(PLUGIN_DIR), "it's me");

    let plugin_dir = Path::new(PLUGIN_DIR);
    fs::write(
        plugin_dir.join("plugin_config.yaml"),
        format!("{SERVICE}:\n  value: \"it's me\"\n"),
    )
    .unwrap();
    let topology_path = plugin_dir.join("topology.toml");
    let topology = fs::read_to_string(&topology_path).unwrap();
    let header = format!("[plugin.{PLUGIN_NAME}]\n");
    assert!(topology.contains(&header));
    fs::write(
        &topology_path,
        topology.replace(
            &header,
            &format!("{header}config = \"plugin_config.yaml\"\n"),
        ),
    )
    .unwrap();

    exec_pike(["run", "--daemon", "--plugin-path", PLUGIN_NAME]);

    assert_plugin_enabled(PLUGIN_DIR, 1);
    let stored = get_picodata_table(plugin_dir, Path::new("tmp/pike"), "_pico_plugin_config");
    assert!(
        stored.contains("it's me") && !stored.contains("it''s me"),
        "{stored}"
    );

    // Smoke check: `config apply` accepts the same file (shared SQL generator), it must not fail.
    exec_pike(["config", "apply", "--plugin-path", PLUGIN_NAME]);
    stop();
}

#[test]
fn run_params_builder_config_path_overrides_topology() {
    init_plugin(PLUGIN_NAME);
    require_config_value_in_on_start(Path::new(PLUGIN_DIR), "changed");

    // The file from topology does not exist, the builder option replaces it.
    let plugin = Plugin {
        config: Some(PathBuf::from("no_such_file.yaml")),
        ..plugin_with_service()
    };
    let topology = topology(BTreeMap::from([(PLUGIN_NAME.to_string(), plugin)]));
    let params = params_builder(topology)
        .add_plugin_config(PLUGIN_NAME, PathBuf::from("plugin_config.yaml"))
        .build()
        .unwrap();
    run(params).unwrap();

    assert_plugin_enabled(PLUGIN_DIR, 1);
    stop();
}

#[test]
fn run_params_builder_config_map() {
    init_plugin(PLUGIN_NAME);
    require_config_value_in_on_start(Path::new(PLUGIN_DIR), "from map");

    let topology = topology(BTreeMap::from([(
        PLUGIN_NAME.to_string(),
        plugin_with_service(),
    )]));
    let params = params_builder(topology)
        .add_plugin_config(PLUGIN_NAME, config_map("from map"))
        .build()
        .unwrap();
    run(params).unwrap();

    assert_plugin_enabled(PLUGIN_DIR, 1);
    stop();
}

/// Without `config` the service gets the defaults from `manifest.yaml.template`,
/// and `plugin_config.yaml` from the plugin template must not be picked up implicitly
#[test]
fn run_without_config_key_uses_manifest_defaults() {
    init_plugin(PLUGIN_NAME);
    // `default_configuration` of `example_service` in the template manifest
    require_config_value_in_on_start(Path::new(PLUGIN_DIR), "example");
    fs::write(
        Path::new(PLUGIN_DIR).join("plugin_config.yaml"),
        format!("{SERVICE}:\n  value: from_file\n"),
    )
    .unwrap();

    let topology = topology(BTreeMap::from([(
        PLUGIN_NAME.to_string(),
        plugin_with_service(),
    )]));
    run(params_builder(topology).build().unwrap()).unwrap();

    assert_plugin_enabled(PLUGIN_DIR, 1);
    let stored = get_picodata_table(
        Path::new(PLUGIN_DIR),
        Path::new("tmp/pike"),
        "_pico_plugin_config",
    );
    assert!(!stored.contains("from_file"), "{stored}");
    stop();
}

#[test]
fn run_fails_early_on_invalid_config() {
    init_plugin(PLUGIN_NAME);

    let cases = [
        // (config file to use, error substring)
        (None, "failed to read config file"),
        (
            Some("unknown_service:\n  value: x\n"),
            "not declared in topology",
        ),
        (Some("not: [valid"), "failed to parse config file"),
    ];

    for (content, expected_err) in cases {
        let file = Path::new(PLUGIN_DIR).join("invalid_config.yaml");
        match content {
            Some(content) => fs::write(&file, content).unwrap(),
            None => drop(fs::remove_file(&file)),
        }
        let plugin = Plugin {
            config: Some(PathBuf::from("invalid_config.yaml")),
            ..plugin_with_service()
        };
        let topology = topology(BTreeMap::from([(PLUGIN_NAME.to_string(), plugin)]));
        let err = run(params_builder(topology).build().unwrap()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(expected_err), "{msg}");

        // no instance has been started
        let cluster_dir = Path::new(PLUGIN_DIR).join("tmp/pike/cluster");
        let started = fs::read_dir(&cluster_dir).into_iter().flatten().any(|e| {
            let dir = e.unwrap().path();
            dir.join("admin.sock").exists() || dir.join("pid").exists()
        });
        assert!(!started, "instance was started despite the invalid config");
    }
}

#[test]
fn run_external_plugin_config_relative_to_topology() {
    init_plugin(PLUGIN_NAME);

    let ext_name = "external-plugin-1";
    let ext_plugin_path = PathBuf::from("./tests/tmp_ext/external-plugin-1");
    init_plugin_with_args(TestPluginInitParams::<String> {
        name: ext_name.to_string(),
        plugin_path: ext_plugin_path.clone(),
        shared_target_path: "./tests/tmp_ext/ext_shared_target".into(),
        working_dir: "./tests/tmp_ext".into(),
        ..Default::default()
    });
    require_config_value_in_on_start(&ext_plugin_path, "from topology dir");
    build_plugin(&helpers::BuildType::Debug, "0.1.0", &ext_plugin_path);

    // topology.toml lives in its own directory, not in cwd
    let topology_dir = Path::new("./tests/tmp_ext/topology_dir");
    fs::create_dir_all(topology_dir).unwrap();
    fs::write(
        topology_dir.join("ext_config.yaml"),
        format!("{SERVICE}:\n  value: from topology dir\n"),
    )
    .unwrap();
    let ext_path = fs::canonicalize(ext_plugin_path.join("target/debug").join(ext_name)).unwrap();
    fs::write(
        topology_dir.join("topology.toml"),
        format!(
            "[tier.default]\nreplicasets = 1\nreplication_factor = 1\n\n\
             [plugin.{ext_name}]\npath = {ext_path:?}\nconfig = \"ext_config.yaml\"\n\n\
             [plugin.{ext_name}.service.{SERVICE}]\ntiers = [\"default\"]\n"
        ),
    )
    .unwrap();

    let mut topology = Topology::parse_toml(&topology_dir.join("topology.toml")).unwrap();
    topology
        .plugins
        .insert(PLUGIN_NAME.to_string(), Plugin::default());
    run(params_builder(topology).build().unwrap()).unwrap();

    assert_plugin_enabled(PLUGIN_DIR, 2);
    stop();
}
