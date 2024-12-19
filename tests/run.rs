mod helpers;

use helpers::run_cluster;
use std::time::Duration;

const TOTAL_INSTANCES: i32 = 4;
#[test]
fn test_cluster_setup_debug() {
    let _cluster_handle = run_cluster(Duration::from_secs(120), TOTAL_INSTANCES).unwrap();
}
