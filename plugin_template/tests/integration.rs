use reqwest::blocking as req;
use picotest::*;

#[picotest]
fn test_cluster_handles() {
    let http_port = cluster.instances().first().unwrap().http_port;

    let resp = req::get(format!("http://127.0.0.1:{http_port}/metrics")).unwrap();
    assert!(resp.status().is_success());

    let resp = req::get(format!("http://127.0.0.1:{http_port}/hello")).unwrap();
    assert!(resp.status().is_success());
}
