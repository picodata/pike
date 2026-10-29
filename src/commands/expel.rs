use crate::commands::lib::{
    find_active_socket_path, get_active_socket_path, get_cluster_dir,
    instance_info::InstanceSocketClient, timeout::Timeout, DEFAULT_DATA_DIR, INSTANCE_PID_FILENAME,
};
use crate::commands::stop::{read_pid_from_file, wait_process_exit};
use anyhow::{bail, Context, Result};
use derive_builder::Builder;
use log::info;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

// Default timeout for an instance to be expelled from the cluster.
pub const DEFAULT_EXPEL_TIMEOUT: Timeout = Timeout::from_secs(60);
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
    timeout: Timeout,
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
    // A running instance is expelled only forcibly, so its process is waited for only then.
    // Pid file of an instance that is not running is stale, so there is no process to wait for.
    let pid = if *force && get_active_socket_path(&cluster_dir, instance_name).is_some() {
        let pid_file = cluster_dir.join(instance_name).join(INSTANCE_PID_FILENAME);
        Some(read_pid_from_file(&pid_file).context("failed to read the PID file")?)
    } else {
        None
    };

    // Expel can be requested through any running instance, including the expelled one.
    let client = find_active_socket_path(&cluster_dir)?
        .map(|socket_path| InstanceSocketClient::with_socket_path(socket_path, picodata_path))
        .context("no running instance found to perform expel")?;

    info!("expelling picodata cluster instance '{instance_name}'");

    client.expel(instance_name, *force, *timeout)?;

    let start = Instant::now();
    loop {
        match client.state_of(instance_name) {
            Ok(state) if state.is_expelled() => break,
            Ok(_) => {}
            // The instance we ask may have gone down, the expelled one for example.
            Err(err) => {
                info!("admin socket is unavailable, stop waiting for '{instance_name}' to become expelled: {err:#}");
                break;
            }
        }

        if start.elapsed() > *timeout {
            bail!("instance '{instance_name}' was not expelled within {timeout:?}");
        }
        sleep(POLL_INTERVAL);
    }

    if let Some(pid) = pid {
        wait_process_exit(pid, *timeout)?;
    }

    info!(
        "instance '{instance_name}' expelled in {:?}",
        start.elapsed()
    );

    Ok(())
}
