use crate::commands::lib::{
    find_active_socket_path, get_cluster_dir, instance_info::InstanceSocketClient,
    resolve_instance_name, timeout::Timeout, DEFAULT_DATA_DIR,
};
use crate::commands::run::INSTANCE_PID_FILENAME;
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
const STATE_TIMEOUT: Duration = Duration::from_mins(5);
const PROCESS_EXIT_TIMEOUT: Timeout = Timeout::from_secs(5 * 60);

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

    let cluster_dir = get_cluster_dir(plugin_path, data_dir);
    let instance_name = resolve_instance_name(&cluster_dir, instance_name)?;
    // Expel can be requested through any running instance, including the expelled one.
    let client = find_active_socket_path(&cluster_dir)?
        .map(|socket_path| InstanceSocketClient::with_socket_path(socket_path, picodata_path))
        .context("no running instance found to perform expel")?;

    let wait_for_process = *force
        && InstanceSocketClient::new(&cluster_dir.join(&instance_name), picodata_path)
            .is_running()?;

    info!("expelling picodata cluster instance '{instance_name}'");

    client.expel(&instance_name, *force, *timeout)?;

    let start = Instant::now();
    loop {
        match client.state_of(&instance_name) {
            Ok(state) if state.is_expelled() => break,
            Ok(_) => {}
            Err(err) if !client.is_running()? => {
                info!("admin socket is unavailable, stop waiting for '{instance_name}' to become expelled: {err:#}");
                break;
            }
            Err(err) => return Err(err),
        }

        if start.elapsed() > STATE_TIMEOUT {
            bail!("instance '{instance_name}' was not expelled within {STATE_TIMEOUT:?}");
        }
        sleep(POLL_INTERVAL);
    }

    if wait_for_process {
        let pid_file = cluster_dir.join(&instance_name).join(INSTANCE_PID_FILENAME);
        let pid = read_pid_from_file(&pid_file).context("failed to read the PID file")?;
        wait_process_exit(pid, PROCESS_EXIT_TIMEOUT)?;
    }

    info!(
        "instance '{instance_name}' expelled in {:?}",
        start.elapsed()
    );

    Ok(())
}
