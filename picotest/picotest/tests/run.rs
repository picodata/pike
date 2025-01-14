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
struct Plugin {}

#[fixture]
pub fn plugin() -> Plugin {
    let mut proc = run_pike(vec!["plugin", "new", PLUGIN_NAME], TMP_DIR).unwrap();
    wait_for_proc(&mut proc, Duration::from_secs(10));
    let _ = build_plugin(PLUGIN_DIR).expect("plugin must be building");
    Plugin {}
}

#[picotest]
fn test_macro(plugin: Plugin) {
    assert_eq!(10, 10)
}

#[picotest]
mod test_mod {
    use crate::{plugin, Plugin};

    fn test_1(plugin: Plugin) {
        assert_eq!(1, 1)
    }

    fn test_2(plugin: Plugin) {
        assert_eq!(2, 2)
    }

    fn test_3(plugin: Plugin) {
        assert_eq!(3, 3)
    }
}

#[picotest]
mod test_mod1 {
    use crate::{plugin, Plugin};

    fn test_4(plugin: Plugin) {
        assert_eq!(3, 3)
    }
}
