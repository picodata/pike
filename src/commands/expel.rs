use crate::commands::lib::{
    find_active_socket_path, get_active_socket_path, get_cluster_dir,
    instance_info::InstanceSocketClient, timeout::Timeout, DEFAULT_DATA_DIR,
};
use crate::commands::stop::{read_pid_from_file, send_signal_and_wait};
use anyhow::{bail, Context, Result};
use derive_builder::Builder;
use log::{debug, info};
use nix::sys::signal::Signal;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

// Default timeout for an instance to be expelled from the cluster.
pub const DEFAULT_EXPEL_TIMEOUT: Duration = Duration::from_secs(60);
// Delay between checks of the instance state.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default = "PathBuf::from(DEFAULT_DATA_DIR)")]
    data_dir: PathBuf,
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default = "PathBuf::from(\"picodata\")")]
    picodata_path: PathBuf,
    instance_name: String,
    /// Expel the instance even if it is still running.
    #[builder(default)]
    force: bool,
    #[builder(default = DEFAULT_EXPEL_TIMEOUT)]
    timeout: Duration,
}

pub fn cmd(params: &Params) -> Result<()> {
    let Params {
        instance_name,
        data_dir,
        plugin_path,
        picodata_path,
        force,
        timeout,
    } = params;

    let cluster_dir = get_cluster_dir(plugin_path, data_dir)
        .canonicalize()
        .context("cluster data dir does not exist")?;
    let (dir_name, picodata_name) = resolve_instance(&cluster_dir, instance_name)?;
    let instance_dir = cluster_dir.join(&dir_name);

    // Pid file of an instance that is not running is stale, so there is no process to wait for.
    let pid = if get_active_socket_path(&cluster_dir, &dir_name).is_some() {
        Some(read_pid_from_file(&instance_dir.join("pid")).context("failed to read the PID file")?)
    } else {
        None
    };

    // Expel can be requested through any running instance, including the expelled one.
    let mut client = find_client(&cluster_dir, picodata_path)?
        .context("no running instance found to perform expel")?;

    info!("expelling picodata cluster instance '{picodata_name}'");

    client.expel(&picodata_name, *force, *timeout)?;

    let start = Instant::now();
    loop {
        match client.state_of(&picodata_name) {
            Ok(state) if state.is_expelled() => break,
            Ok(_) => {}
            // The instance we ask may have gone down, the expelled one for example: ask another.
            Err(err) => {
                debug!("failed to get state of '{picodata_name}': {err:#}");
                client = find_client(&cluster_dir, picodata_path)?.with_context(|| {
                    format!(
                        "no running instance left to check the state of '{picodata_name}': {err:#}"
                    )
                })?;
            }
        }

        if start.elapsed() > *timeout {
            bail!("instance '{picodata_name}' was not expelled within {timeout:?}");
        }
        sleep(POLL_INTERVAL);
    }

    if let Some(pid) = pid {
        send_signal_and_wait(pid, Signal::SIGTERM, Timeout::from(*timeout))?;
    }

    info!(
        "instance '{picodata_name}' expelled in {:?}",
        start.elapsed()
    );

    Ok(())
}

/// Returns a client of any running instance of the cluster.
fn find_client<'a>(
    cluster_dir: &Path,
    picodata_path: &'a PathBuf,
) -> Result<Option<InstanceSocketClient<'a>>> {
    let Some(socket_path) = find_active_socket_path(cluster_dir)? else {
        return Ok(None);
    };
    let socket_dir = socket_path
        .parent()
        .context("admin socket path has no parent")?;

    Ok(Some(InstanceSocketClient::new(socket_dir, picodata_path)))
}

/// Returns the name of the instance directory and the picodata name of the instance
/// by either of them.
fn resolve_instance(cluster_dir: &Path, instance_name: &str) -> Result<(String, String)> {
    let instance_dir = cluster_dir
        .join(instance_name)
        .canonicalize()
        .with_context(|| format!("failed to locate directory of the instance '{instance_name}'"))?;
    if instance_dir.parent() != Some(cluster_dir) {
        bail!("'{instance_name}' is not an instance of the cluster");
    }

    let dir_name = instance_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("instance directory name contains invalid characters")?
        .to_owned();

    for entry in fs::read_dir(cluster_dir)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.is_symlink() && path.canonicalize()? == instance_dir {
            let picodata_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .context("instance name contains invalid characters")?
                .to_owned();
            return Ok((dir_name, picodata_name));
        }
    }

    bail!("instance '{instance_name}' has never become online, it has no picodata name to expel")
}
