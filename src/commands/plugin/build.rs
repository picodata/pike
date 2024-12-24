use anyhow::{Context, Result};
use lib::{cargo_build, BuildType};

use crate::commands::lib;

pub fn cmd(release: bool) -> Result<()> {
    let build_type = if release {
        BuildType::Release
    } else {
        BuildType::Debug
    };
    cargo_build(build_type).context("building of plugin")
}
