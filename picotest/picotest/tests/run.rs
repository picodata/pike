use constcat::concat;
use picotest::*;
use rstest::*;

pub const TESTS_DIR: &str = "./tests/tmp";
pub const PLUGIN_DIR: &str = concat!(TESTS_DIR, "test_plugin/");

#[derive(Debug)]
struct Plugin {
    name: String,
}

impl Default for Plugin {
    fn default() -> Self {
        Self {
            name: "test_plugin".to_string(),
        }
    }
}

#[fixture]
fn test_plugin() -> Plugin {
    println!("CREATE PLUGIN");
    let mut plugin_creation_proc =
        run_pike(vec!["plugin", "new", "test_plugin"], TESTS_DIR).unwrap();
    Default::default()
}

#[picotest]
fn test_macro(test_plugin: Plugin) {
    println!("PLUGIN: {:?}", test_plugin);
}
