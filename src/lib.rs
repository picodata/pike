#![allow(dead_code, clippy::missing_errors_doc, clippy::missing_panics_doc)]
mod commands;

pub mod cluster {
    pub use crate::commands::run::cluster as run;
    pub use crate::commands::run::ParamsBuilder as RunParamsBuilder;

    pub use crate::commands::run::MigrationContextVar;
    pub use crate::commands::run::PicodataInstance;
    pub use crate::commands::run::Plugin;
    pub use crate::commands::run::Service;
    pub use crate::commands::run::Tier;
    pub use crate::commands::run::Topology;

    pub use crate::commands::stop::cmd as stop;
    pub use crate::commands::stop::ParamsBuilder as StopParamsBuilder;
}

pub mod config {
    pub use crate::commands::config::apply::cmd as apply;
    pub use crate::commands::config::apply::ConfigMap as PluginConfigMap;
    pub use crate::commands::config::apply::ParamsBuilder as ApplyParamsBuilder;
}

pub mod helpers;
