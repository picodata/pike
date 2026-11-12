use super::merge;
use super::model::{InstancesStatus, ProcessState};
use super::Params;
use crate::commands::lib::{
    get_active_socket_path, get_cluster_dir,
    instance_info::{ClusterInstance, InstanceSocketClient},
};
use crate::commands::run::{PicodataInstance, Topology, INSTANCE_PID_FILENAME};
use crate::commands::stop::read_pid_from_file;
use anyhow::{Context, Result};
use log::warn;
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// An instance whose directory is found in the cluster directory
#[derive(Debug)]
pub(super) struct DirectoryInstance {
    pub(super) n: u16,
    /// Process state by PID file
    pub(super) process: ProcessState,
    /// Name of the picodata instance: symlink pointing at the `i<N>` directory
    pub(super) instance_name: Option<String>,
}

impl DirectoryInstance {
    pub(super) fn is_running(&self) -> bool {
        self.process == ProcessState::Running
    }
}

/// Collects the instance status: the instances expected from the topology,
/// the instance directories found in the cluster directory, and the cluster
/// data of the first running instance that answers.
pub fn collect(params: &Params) -> Result<InstancesStatus> {
    let cluster_dir = get_cluster_dir(&params.plugin_path, &params.data_dir);
    // consider instances from topology.toml as expected
    let expected = extract_expected_instances(&params.topology);
    // find the actual instances in the cluster directory by its subdirectories
    let actual = scan_cluster_dir(&cluster_dir)?;
    // fetch data from picodata system tables/functions
    let cluster_data = fetch_cluster_data(&cluster_dir, &actual, &params.picodata_path);

    // merge the expected instances with the found ones, enriching the result
    // with cluster data, joined on the instance name from the symlink
    Ok(merge::merge(&expected, &actual, cluster_data.as_deref()))
}

/// Fetches the cluster data from the system tables/functions via the first
/// running instance whose admin socket answers the query. Returns `None`
/// without a warning when nothing runs (a stopped cluster is a normal state)
/// and logs a warning when something runs but no query succeeded.
pub(super) fn fetch_cluster_data(
    cluster_dir: &Path,
    found_on_disk: &[DirectoryInstance],
    picodata_path: &PathBuf,
) -> Option<Vec<ClusterInstance>> {
    let mut running: Vec<u16> = found_on_disk
        .iter()
        .filter(|instance| instance.is_running())
        .map(|instance| instance.n)
        .collect();
    running.sort_unstable();

    if running.is_empty() {
        // means the cluster is stopped or has never been started
        return None;
    }

    // try to fetch data from first running instance that answers
    let mut last_error = None;
    for n in &running {
        let dir = PicodataInstance::make_data_dir_name(*n);
        let Some(socket_path) = get_active_socket_path(cluster_dir, &dir) else {
            continue;
        };

        let client = InstanceSocketClient::with_socket_path(socket_path, picodata_path);
        match client.cluster_instances() {
            Ok(data) => return Some(data),
            Err(error) => last_error = Some(error),
        }
    }

    // log an error only if we could not get a picodata answer
    let reason = last_error.map_or(
        "no running instance has an active admin socket".to_string(),
        |error| format!("{error:#}"),
    );
    warn!("failed to fetch cluster data from any running instance: {reason}. running instances ({}): {running:?}", running.len());

    None
}

/// Instance directory numbers (`i<N>`) with their tier from topology.toml
pub(super) fn extract_expected_instances(topology: &Topology) -> Vec<(u16, String)> {
    topology
        .tier_instance_ids()
        .into_iter()
        .flat_map(|(tier_name, instance_ids)| {
            instance_ids.map(move |id| (id, tier_name.to_string()))
        })
        .collect()
}

/// Scan the cluster directory: instance directories `i<N>` with their state (by PID file)
/// and Picodata name symlinks (for example `default_1_1`) pointing at them.
pub(super) fn scan_cluster_dir(cluster_dir: &Path) -> Result<Vec<DirectoryInstance>> {
    let subdirs = match fs::read_dir(cluster_dir) {
        Ok(sd) => sd,
        // if the cluster directory does not exist we consider the instances not started
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(e).context(format!(
                "failed to read the cluster directory {}",
                cluster_dir.display()
            ))
        }
    };

    // `i<N>` (directory name) -> instance name (symlink). for example: {"i1": "default_1_1"}
    let mut names: HashMap<String, String> = HashMap::new();
    // for example: [(1, "i1"), .., (n, "in")]
    let mut instance_dirs = Vec::new();

    for instance_dir in subdirs {
        let instance_dir = instance_dir?;
        // skip entries with non-UTF-8 names
        let Ok(file_name) = instance_dir.file_name().into_string() else {
            continue;
        };

        // type can be symlink, directory or file
        let file_type = instance_dir.file_type()?;

        // separate symlinks and instance directories
        if file_type.is_symlink() {
            let target = match fs::read_link(instance_dir.path()) {
                Ok(target) => target,
                // can happen if the symlink has gone missing between reading dir and link
                Err(e) if e.kind() == ErrorKind::NotFound => continue,
                Err(e) => {
                    warn!(
                        "failed to read the instance name symlink {}: {e}",
                        instance_dir.path().display()
                    );
                    continue;
                }
            };
            // Debug prints the invalid bytes as \xNN escapes, unlike the lossy Display
            #[allow(clippy::unnecessary_debug_formatting)]
            let target = target.into_string().unwrap_or_else(|target| {
                panic!(
                    "unreachable: symlink {} has non-UTF-8 target {:?}",
                    instance_dir.path().display(),
                    target
                )
            });
            names.insert(target, file_name);
        } else if file_type.is_dir() {
            if let Some(n) = parse_instance_n(&file_name) {
                instance_dirs.push((n, file_name));
            }
        }
    }

    let found_on_disk = instance_dirs
        .into_iter()
        .map(|(n, dir_name)| DirectoryInstance {
            n,
            process: ProcessState::from(cluster_dir.join(&dir_name).as_path()),
            instance_name: names.get(&dir_name).cloned(),
        })
        .collect();

    Ok(found_on_disk)
}

/// `i<N>` directory name to `N`, `None` for anything else.
pub(super) fn parse_instance_n(file_name: &str) -> Option<u16> {
    file_name.strip_prefix('i')?.parse().ok()
}

/// Process state of an instance directory by its `pid` file. Missing/unreadable
/// pid file counts as stopped.
///
/// A pid file holding a non-positive value panics instead of counting as stopped.
/// The assert is pre-existing and shared with `stop`/`expel` (turning it into an
/// error would change the behavior of those commands). The case is unreachable
/// either way: pid files are written by `run` itself and always hold a real child pid.
impl From<&Path> for ProcessState {
    fn from(instance_dir: &Path) -> Self {
        match read_pid_from_file(&instance_dir.join(INSTANCE_PID_FILENAME)) {
            Ok(pid) if is_process_alive(pid) => Self::Running,
            _ => Self::Stopped,
        }
    }
}

/// Check that the process is alive and not a zombie.
fn is_process_alive(pid: Pid) -> bool {
    let exists = match kill(pid, None) {
        // EPERM: the process exists, but belongs to another user.
        Ok(()) | Err(Errno::EPERM) => true,
        Err(_) => false,
    };
    if !exists {
        return false;
    }

    // On other platforms a zombie is considered alive (same behavior as `stop`)
    #[cfg(target_os = "linux")]
    if crate::commands::stop::is_zombie(pid) {
        return false;
    }

    true
}
