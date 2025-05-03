use picodata_plugin::transport::rpc;
use picodata_plugin::{plugin::prelude::*, transport::rpc::RouteBuilder};
use serde::{Deserialize, Serialize};

use crate::models::User;

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleResponse {
    pub rpc_hello_response: String,
}

pub fn register_example_rpc_handle(context: &PicoContext) {
    RouteBuilder::from_pico_context(context)
        .path("/greetings_rpc")
        .register(move |req, _ctx| {
            log::debug!("Received store request: {req:?}");

            let user: User = rmp_serde::from_slice(req.as_bytes()).unwrap();

            log::warn!("Recieved \"{user:?}\" as RPC input");

            let user_name = user.name;
            let response_to_return = ExampleResponse {
                rpc_hello_response: format!("Hello {user_name}, long time no see."),
            };

            Ok(rpc::Response::encode_rmp(&response_to_return).unwrap())
        })
        .unwrap();
}
