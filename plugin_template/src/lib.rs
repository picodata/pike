mod config;
mod handlers;
mod models;
mod service;

use picodata_plugin::plugin::{interface::ServiceRegistry, prelude::service_registrar};

#[service_registrar]
pub fn service_registrar(reg: &mut ServiceRegistry) {
    reg.add(
        "example_service",
        env!("CARGO_PKG_VERSION"),
        service::ExampleService::default,
    );
    reg.add_config_validator::<service::ExampleService>("example_service", env!("CARGO_PKG_VERSION"), |cfg| {
        if let Some(cfg_value) = cfg.value {
            if cfg_value == "tarantool" {
                return Err("Please call a pest control service!".into());
            }
        }
        Ok(())
    });
}
