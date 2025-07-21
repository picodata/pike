use crate::commands::lib::get_active_socket_path;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use derive_builder::Builder;
use log::info;
use std::fs::{self};
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default = "PathBuf::from(\"./tmp\")")]
    data_dir: PathBuf,
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default)]
    instance_name: Option<String>,
}

pub fn cmd(params: &Params) -> Result<()> {
    let instances_path = params.plugin_path.join(params.data_dir.join("cluster"));
    let dirs = fs::read_dir(&instances_path).context(format!(
        "cluster data dir with path {} does not exist",
        instances_path.to_string_lossy()
    ))?;

    if let Some(instance_name) = &params.instance_name {
        info!(
            "stopping picodata cluster instance '{instance_name}', data folder: {}",
            instances_path.join(instance_name).to_string_lossy()
        );

        // Find directory that belongs to stopping instance.
        let instance_dir = dirs.into_iter().find_map(|result| {
            let Ok(dir_entry) = result else {
                return None;
            };

            if dir_entry.file_name() != instance_name.as_str() {
                return None;
            }

            Some(dir_entry.path())
        });

        let Some(instance_dir) = instance_dir else {
            bail!("failed to locate directory of the instance '{instance_name}'");
        };

        stop_instance(params, &instance_dir)
    } else {
        info!(
            "stopping picodata cluster, data folder: {}",
            params.data_dir.to_string_lossy()
        );

        // Iterate through instance folders and
        // search for "pid" file. After the pid
        // is known - kill the instance
        for current_dir in dirs {
            let instance_dir = current_dir?.path();

            // To get the actual instance name, we look
            // only on simlinks
            if !fs::symlink_metadata(&instance_dir)?.is_symlink() {
                continue;
            }

            stop_instance(params, &instance_dir)?;
        }

        Ok(())
    }
}

fn stop_instance(params: &Params, instance_dir: &Path) -> Result<()> {
    if !instance_dir.is_dir() {
        bail!("{} is not a directory", instance_dir.to_string_lossy());
    }
    let Some(link_name) = instance_dir.file_name() else {
        bail!(
            "failed to obtain filename from {}",
            instance_dir.to_string_lossy()
        );
    };

    let pid_file_path = instance_dir.join("pid");
    if !pid_file_path.exists() {
        bail!(
            "PID file does not exist in folder: {}",
            instance_dir.display()
        );
    }

    let pid = read_pid_from_file(&pid_file_path).context("failed to read the PID file")?;

    if get_active_socket_path(
        &params.data_dir,
        &params.plugin_path,
        link_name.to_str().unwrap(),
    )
    .is_none()
    {
        info!(
            "stopping picodata instance: {} - {}",
            link_name.to_string_lossy(),
            "SKIPPED".yellow()
        );
        return Ok(());
    }

    if let Err(e) = kill_process_by_pid(pid) {
        bail!("failed to stop picodata instance with PID {pid}. Error: {e}");
    }
    info!(
        "stopping picodata instance: {} - {}",
        link_name.to_string_lossy(),
        "OK".green()
    );

    Ok(())
}

fn read_pid_from_file(pid_file_path: &Path) -> Result<u32> {
    let file = fs::File::open(pid_file_path)?;

    let mut lines = io::BufReader::new(file).lines();
    let pid_line = lines.next().context("PID file is empty")??;

    let pid = pid_line.trim().parse::<u32>().context(format!(
        "failed to parse PID from file {}",
        pid_file_path.display()
    ))?;

    Ok(pid)
}

fn kill_process_by_pid(pid: u32) -> Result<()> {
    let output = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()?;

    if !output.status.success() {
        bail!("failed to kill picodata instance (pid: {pid}): {output:?}");
    }

    Ok(())
}
