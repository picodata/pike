use crate::config;
use crate::handlers::endpoints::http::routes;

use crate::handlers::endpoints::rpc::register_example_rpc_handle;
use picodata_plugin::plugin::prelude::*;
use picodata_plugin::system::tarantool::log as t_log;
use shors::transport::http::server::Server;
use std::sync::LazyLock;

static LOGGER: LazyLock<t_log::TarantoolLogger> = LazyLock::new(t_log::TarantoolLogger::default);

fn init_logger() {
    log::set_logger(&*LOGGER).map_or_else(
        |e| println!("failed to setup logger: {e:?}"),
        |()| log::set_max_level(log::LevelFilter::Trace),
    );
}

thread_local! {
    static HTTP_SERVER: LazyLock<Server> = LazyLock::new(Server::new);
}

#[derive(Debug, Default)]
pub struct ExampleService {}

impl Service for ExampleService {
    type Config = config::ExampleService;

    fn on_config_change(
        &mut self,
        ctx: &PicoContext,
        new_config: Self::Config,
        old_config: Self::Config,
    ) -> CallbackResult<()> {
        _ = ctx;
        _ = new_config;
        _ = old_config;

        Ok(())
    }

    fn on_start(&mut self, context: &PicoContext, config: Self::Config) -> CallbackResult<()> {
        _ = context;
        _ = config;

        init_logger();

        log::warn!("Registering HTTP handle /hello");
        HTTP_SERVER.with(|srv| {
            routes()
                .into_iter()
                .for_each(|route| srv.register(Box::new(route)));
        });

        log::warn!("Registering RPC handle /greetings_rpc");
        register_example_rpc_handle(context);

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
