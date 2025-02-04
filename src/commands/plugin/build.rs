use std::path::PathBuf;

use anyhow::{Context, Result};
use lib::{cargo_build, BuildType};

use crate::commands::lib;

pub fn cmd(release: bool, target_dir: &PathBuf) -> Result<()> {
    let build_type = if release {
        BuildType::Release
    } else {
        BuildType::Debug
    };
    cargo_build(build_type, target_dir).context("building of plugin")
}
