mod collector;
mod merge;
pub(crate) mod model;
mod render;

#[cfg(test)]
mod test;

pub use collector::collect;

use crate::commands::run::Topology;
use anyhow::Result;
use clap::ValueEnum;
use derive_builder::Builder;
use std::path::PathBuf;

use crate::commands::lib::DEFAULT_DATA_DIR;
use std::fmt::{self, Display, Formatter};

#[derive(Debug, Builder)]
pub struct Params {
    topology: Topology,
    #[builder(default = "PathBuf::from(DEFAULT_DATA_DIR)")]
    data_dir: PathBuf,
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default = "PathBuf::from(\"picodata\")")]
    picodata_path: PathBuf,
    #[builder(default)]
    format: OutputFormat,
}

pub fn cmd(params: &Params) -> Result<()> {
    let output = collect(params)?;

    // render the output
    match params.format {
        OutputFormat::Text => println!("{}", render::render_text(&output)),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&output)?),
    }
    Ok(())
}

/// Output format of the `status instances` command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Text => "text",
            Self::Json => "json",
        })
    }
}
