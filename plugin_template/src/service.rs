use std::collections::HashMap;

use crate::config;

use anyhow;
use once_cell::sync::Lazy;
use picodata_plugin::plugin::prelude::*;
use shors::transport::{
    http::{
        route::{Builder, Route},
        server::Server,
        Request, Response,
    },
    Context,
};

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

        HTTP_SERVER.with(|srv| {
            routes()
                .into_iter()
                .for_each(|route| srv.register(Box::new(route)));
        });

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

pub fn routes() -> Vec<Route<anyhow::Error>> {
    let hello_route = Builder::new().with_method("GET").with_path("/hello").build(
        |_: &mut Context, _: Request| -> anyhow::Result<_> {
            let message: &str = r"
     _________________________________
    / Hello there! I see that you've  \
    | successfully deployed picodata  |
    | app using pike. Congrats! Type  |
    \ cargo pike --help for more tips /
     ---------------------------------
                    |
                    |
                   ,|.
                  ,\|/.
                ,' .V. `.
               / .     . \
              /_`       '_\
             ,' .:     ;, `.
             |@)|  . .  |(@|
        ,-._ `._';  .  :`_,' _,-.
       '--  `-\ /,-===-.\ /-'  --`
      (----  _|  ||___||  |_  ----)
       `._,-'  \  `-.-'  /  `-._,'
                `-.___,-'
     ";
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
