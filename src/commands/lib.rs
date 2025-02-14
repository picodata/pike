use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub enum BuildType {
    Release,
    Debug,
}

#[allow(clippy::needless_pass_by_value)]
pub fn cargo_build(build_type: BuildType, target_dir: &PathBuf, build_dir: &PathBuf) -> Result<()> {
    let mut args = vec!["build"];
    if let BuildType::Release = build_type {
        args.push("--release");
    }

    let mut child = Command::new("cargo")
        .args(args)
        .arg("--target-dir")
        .arg(target_dir)
        .stdout(Stdio::piped())
        .current_dir(build_dir)
        .spawn()
        .context("running cargo build")?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = line.unwrap_or_else(|e| format!("{e}"));
        print!("{line}");
    }

    if !child.wait().unwrap().success() {
        let mut stderr = String::new();
        child.stderr.unwrap().read_to_string(&mut stderr).unwrap();
        bail!("build error: {stderr}");
    }

    Ok(())
}
