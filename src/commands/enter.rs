use anyhow::{bail, Context, Result};
use log::info;
use std::{path::Path, process::Command};

pub fn cmd(
    instance_name: &str,
    data_dir: &Path,
    plugin_path: &Path,
    picodata_path: &Path,
) -> Result<()> {
    info!("Entering instance <{instance_name}>");

    let cluster_dir = plugin_path.join(data_dir).join("cluster");

    enter_instance(&cluster_dir, instance_name, picodata_path)
        .context(format!("failed to enter instance {instance_name}"))
}

fn enter_instance(base_path: &Path, instance_name: &str, picodata_path: &Path) -> Result<()> {
    let instance_dir_path = base_path.join(instance_name);
    if !instance_dir_path.exists() || !instance_dir_path.is_dir() {
        bail!(
            "failed to find instance data directory with path {}",
            instance_dir_path.display()
        )
    }

    let sock_path = instance_dir_path.join("admin.sock");
    if !sock_path.exists() {
        bail!("failed to find admin.sock in instance directory");
    }

    let status = Command::new(picodata_path)
        .arg("admin")
        .arg(sock_path.to_str().unwrap())
        .status()
        .context("failed to execute picodata")?;
    if !status.success() {
        bail!("failed to execute picodata admin");
    }

    Ok(())
}
