mod helpers;

use constcat::concat;
use helpers::{build_plugin, wait_for_proc};
use picotest::*;
use rstest::*;
use std::time::Duration;

pub const TMP_DIR: &str = "../tmp/";
pub const PLUGIN_NAME: &str = "test_plugin";
pub const PLUGIN_DIR: &str = concat!(TMP_DIR, PLUGIN_NAME);

#[derive(Debug)]
struct Plugin {
    name: String,
}

#[fixture]
#[once]
pub fn plugin() -> Plugin {
    let mut proc = run_pike(vec!["plugin", "new", PLUGIN_NAME], TMP_DIR).unwrap();
    wait_for_proc(&mut proc, Duration::from_secs(10));
    let _ = build_plugin(PLUGIN_DIR).expect("plugin must be building");
    Plugin {
        name: PLUGIN_NAME.to_string(),
    }
}

#[picotest(path = "../tmp/test_plugin")]
fn test_func_install_plugin(plugin: &Plugin) {
    let enabled = cluster.run_query(format!(
        r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
        plugin.name
    ));
    assert!(enabled.is_ok());
    assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
}

#[picotest(path = "../tmp/test_plugin")]
mod test_mod {
    use crate::{plugin, Plugin};
    use std::sync::OnceLock;
    use uuid::Uuid;

    static CLUSTER_UUID: OnceLock<Uuid> = OnceLock::new();

    fn test_once_cluster_1(plugin: &Plugin) {
        let cluster_uuid = CLUSTER_UUID.get_or_init(|| cluster.uuid);
        assert_eq!(cluster_uuid, &cluster.uuid);

        let enabled = cluster.run_query(format!(
            r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
            plugin.name
        ));
        assert!(enabled.is_ok());
        assert!(enabled.is_ok_and(|enabled| enabled.contains("true")))
    }

    fn test_once_cluster_2(plugin: &Plugin) {
        let cluster_uuid = CLUSTER_UUID.get_or_init(|| cluster.uuid);
        assert_eq!(cluster_uuid, &cluster.uuid);

        let enabled = cluster.run_query(format!(
            r#"SELECT enabled FROM _pico_plugin WHERE name = '{}';"#,
            plugin.name
        ));
        assert!(enabled.is_ok());
        assert!(enabled.is_ok_and(|enabled| enabled.contains("true")));
    }
}
