use std::collections::HashMap;

use crate::config;

use once_cell::sync::Lazy;
use picodata_plugin::system::tarantool::log as t_log;
use picodata_plugin::transport::rpc;
use picodata_plugin::{plugin::prelude::*, transport::rpc::RouteBuilder};
use serde::{Deserialize, Serialize};
use shors::transport::{
    Context,
    http::{
        Request, Response,
        route::{Builder, Route},
        server::Server,
    },
};
use std::sync::LazyLock;

#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExampleResponse {
    data: String,
}

static LOGGER: LazyLock<t_log::TarantoolLogger> = LazyLock::new(t_log::TarantoolLogger::default);

fn init_logger() {
    log::set_logger(&*LOGGER).map_or_else(
        |e| println!("failed to setup logger: {e:?}"),
        |()| log::set_max_level(log::LevelFilter::Trace),
    );
}

thread_local! {
    static HTTP_SERVER: Lazy<Server> = Lazy::new(Server::new);
}

#[derive(Debug)]
pub struct PluginService {}

impl Default for PluginService {
    fn default() -> Self {
        Self {}
    }
}

impl Service for PluginService {
    type Config = Option<config::Config>;

    fn on_config_change(
        &mut self,
        ctx: &PicoContext,
        new_config: Self::Config,
        old_config: Self::Config,
    ) -> CallbackResult<()> {
        _ = ctx;
        _ = new_config.unwrap_or_default();
        _ = old_config.unwrap_or_default();

        Ok(())
    }

    fn on_start(&mut self, context: &PicoContext, config: Self::Config) -> CallbackResult<()> {
        _ = context;
        _ = config.unwrap_or_default();

        init_logger();
        log::warn!("Registering HTTP handle /hello");

        HTTP_SERVER.with(|srv| {
            routes()
                .into_iter()
                .for_each(|route| srv.register(Box::new(route)));
        });

        log::warn!("Registering RPC handle /test");

        RouteBuilder::from_pico_context(context)
            .path("/test")
            .register(move |req, _ctx| {
                log::debug!("Received store request: {req:?}");

                let user: User = rmp_serde::from_slice(req.as_bytes()).unwrap();
                log::warn!("Recieved \"{user:?}\" as RPC input");

                let user_name = user.name;
                let response_to_return = ExampleResponse {
                    data: format!("Hello {user_name}, long time no see."),
                };

                Ok(rpc::Response::encode_rmp(&response_to_return).unwrap())
            })
            .unwrap();

        Ok(())
    }

    fn on_stop(&mut self, context: &PicoContext) -> CallbackResult<()> {
        _ = context;
        Ok(())
    }

    fn on_leader_change(&mut self, context: &PicoContext) -> CallbackResult<()> {
        _ = context;
        Ok(())
    }

    fn on_health_check(&self, context: &PicoContext) -> CallbackResult<()> {
        _ = context;
        Ok(())
    }
}

#[must_use]
pub fn routes() -> Vec<Route<anyhow::Error>> {
    let hello_route = Builder::new().with_method("GET").with_path("/hello").build(
        |_: &mut Context, _: Request| -> anyhow::Result<_> {
            let message: &str = "Hello there! This is pike. Use cargo pike --help for more tips.";
            Ok(Response {
                status: 200,
                headers: HashMap::from([(
                    "content-type".to_string(),
                    "application/json; charset=utf8".to_string(),
                )]),
                body: message.as_bytes().to_vec(),
            })
        },
    );

    vec![hello_route]
}
