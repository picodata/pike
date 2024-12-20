mod helpers;

use helpers::run_cluster;
use reqwest::blocking as req;

#[test]
fn test_metrics() {
    let _cluster_handle = run_cluster();
    let resp = req::get("http://localhost:8001/metrics").unwrap();
    assert!(resp.status().is_success());
}
