use crate::commands::lib::{
    find_active_socket_path, get_cluster_dir, instance_info::InstanceSocketClient,
};
use crate::commands::stop;
use anyhow::{bail, Context, Result};
use derive_builder::Builder;
use log::info;
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
    #[builder(default = "PathBuf::from(\"./tmp\")")]
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

    // An instance can not expel itself, so ask any other live one.
    let peer_socket = find_active_socket_path(&cluster_dir, Some(&dir_name))?
        .context("no running instance left to perform expel")?;
    let peer_dir = peer_socket
        .parent()
        .context("admin socket path has no parent")?;
    let peer = InstanceSocketClient::new(peer_dir, picodata_path);

    info!("expelling picodata cluster instance '{picodata_name}'");

    peer.expel(&picodata_name, *force, *timeout)?;

    let start = Instant::now();
    while !peer.state_of(&picodata_name)?.is_expelled() {
        if start.elapsed() > *timeout {
            bail!("instance '{picodata_name}' was not expelled within {timeout:?}");
        }
        sleep(POLL_INTERVAL);
    }

    // Expelled instance shuts itself down, but make sure the process is gone.
    let stop_params = stop::ParamsBuilder::default()
        .data_dir(data_dir.clone())
        .plugin_path(plugin_path.clone())
        .instance_name(Some(dir_name))
        .build()?;
    stop::cmd(&stop_params)?;

    info!(
        "instance '{picodata_name}' expelled in {:?}",
        start.elapsed()
    );

    Ok(())
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
