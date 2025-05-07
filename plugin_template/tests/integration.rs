use picotest::*;
use reqwest::blocking as req;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleResponse {
    rpc_hello_response: String,
}

#[picotest]
fn test_cluster_handles() {
    let http_port = cluster.main().http_port;

    let resp = req::get(format!("http://127.0.0.1:{http_port}/metrics")).unwrap();
    assert!(resp.status().is_success());

    let resp = req::get(format!("http://127.0.0.1:{http_port}/hello")).unwrap();
    assert!(resp.status().is_success());
}

#[tokio::test]
#[picotest]
async fn test_rpc_handle() {
    let user_to_send = User {
        name: "Dodo".to_string(),
    };

    let tnt_response = cluster
        .main()
        .execute_rpc::<User, ExampleResponse>(
            env!("CARGO_PKG_NAME"),
            "/greetings_rpc",
            "main",
            env!("CARGO_PKG_VERSION"),
            &user_to_send,
        )
        .await
        .unwrap();

    assert_eq!(
        tnt_response.rpc_hello_response,
        "Hello Dodo, long time no see."
    );
}
