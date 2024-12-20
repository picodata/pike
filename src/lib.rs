#![allow(dead_code, clippy::missing_errors_doc, clippy::missing_panics_doc)]
mod commands;

pub mod cluster {
    pub use crate::commands::run::cluster as run;
    pub use crate::commands::stop::cmd as stop;
}

pub mod config {
    pub use crate::commands::config::apply::cmd as apply;
}
