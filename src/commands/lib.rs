use anyhow::{bail, Context, Result};
use std::process::Command;

pub enum BuildType {
    Release,
    Debug,
}

#[allow(clippy::needless_pass_by_value)]
pub fn cargo_build(build_type: BuildType) -> Result<()> {
    let output = match build_type {
        BuildType::Release => Command::new("cargo")
            .args(["build", "--release"])
            .output()
            .context("running cargo build")?,
        BuildType::Debug => Command::new("cargo")
            .arg("build")
            .output()
            .context("running cargo build")?,
    };

    if !output.status.success() {
        bail!("build error: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(())
}
