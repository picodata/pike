use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
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

// Return socket path to active instance
pub fn get_active_socket_path(
    data_dir: &Path,
    plugin_path: &Path,
    instance_name: &str,
) -> Option<String> {
    let socket_path = plugin_path
        .join(data_dir)
        .join("cluster")
        .join(instance_name)
        .join("admin.sock");

    if socket_path.exists() && UnixStream::connect(&socket_path).is_ok() {
        return socket_path.to_str().map(str::to_owned);
    }

    None
}

// Scan data directory and return the first active instance's socket path
pub fn check_running_instances(data_dir: &Path, plugin_path: &Path) -> Result<Option<String>> {
    let instances_path = plugin_path.join(data_dir.join("cluster"));
    if !instances_path.exists() {
        return Ok(None);
    }

    let dirs = fs::read_dir(&instances_path).context(format!(
        "cluster data dir with path {} does not exist",
        instances_path.to_string_lossy()
    ))?;

    for current_dir in dirs {
        let dir_name = current_dir?.file_name();
        if let Some(name) = dir_name.to_str() {
            let instance_name = get_active_socket_path(data_dir, plugin_path, name);
            if instance_name.is_some() {
                return Ok(instance_name);
            }
        }
    }

    Ok(None)
}
