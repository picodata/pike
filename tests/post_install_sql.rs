mod helpers;

use helpers::{get_picodata_table, init_plugin, PLUGIN_DIR, PLUGIN_NAME};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

use pike::cluster::run;
use pike::cluster::Plugin;
use pike::cluster::RunParamsBuilder;
use pike::cluster::Tier;
use pike::cluster::Topology;

#[test]
fn test_post_install_sql_execution() {
    init_plugin(PLUGIN_NAME);

    let plugin_path = Path::new(PLUGIN_DIR);

    let tiers = BTreeMap::from([(
        "default".to_string(),
        Tier {
            replicasets: 1,
            replication_factor: 2,
        },
    )]);

    let mut plugins = BTreeMap::new();
    plugins.insert(PLUGIN_NAME.to_string(), Plugin::default());

    // 'using ldap' to avoid false positives by security scanners on
    // "embedded password in the code"
    let topology = Topology {
        tiers,
        plugins,
        enviroment: BTreeMap::new(),
        pre_install_sql: vec![],
        post_install_sql: vec![r#"CREATE USER "post_install_user" USING ldap;"#.to_string()],
    };

    let params = RunParamsBuilder::default()
        .topology(topology)
        .daemon(true)
        .plugin_path(plugin_path.to_path_buf())
        .build()
        .unwrap();

    let _instances = run(params).expect("Cluster run failed");

    let start = Instant::now();
    let mut check_passed = false;

    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let result = std::panic::catch_unwind(|| {
            get_picodata_table(plugin_path, Path::new("tmp"), "_pico_user")
        });

        if let Ok(output) = result {
            if output.contains("post_install_user") {
                check_passed = true;
                break;
            }
        }

        std::thread::sleep(Duration::from_secs(1));
    }

    pike::cluster::stop(
        &pike::cluster::StopParamsBuilder::default()
            .plugin_path(plugin_path.to_path_buf())
            .build()
            .unwrap(),
    )
    .unwrap();

    assert!(
        check_passed,
        "Post-install SQL scripts were not executed or user is missing"
    );
}
