mod helpers;

use helpers::{exec_pike, get_picodata_table, run_cluster, CmdArguments, PLUGIN_DIR};
use std::{
    path::Path,
    time::{Duration, Instant},
    vec,
};

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
