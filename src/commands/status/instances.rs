use crate::commands::lib::instance_info::{ClusterInstances, InstanceSocketClient};
use crate::commands::lib::{
    get_active_socket_path, get_cluster_dir, read_pid_file, DEFAULT_DATA_DIR,
};
#[cfg(target_os = "linux")]
use crate::commands::lib::{is_zombie, resolve_instance_dir_of_process_from_cmdline_and_cwd};
use crate::commands::run::{PicodataInstance, Topology};
use anyhow::{bail, Context, Result};
use derive_builder::Builder;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsStr;
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use std::{fmt, fs, iter};

/// Options of `status instances`.
#[derive(Debug, Builder)]
pub struct Params {
    topology: Topology,
    #[builder(default = "PathBuf::from(DEFAULT_DATA_DIR)")]
    data_dir: PathBuf,
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default = "PathBuf::from(\"picodata\")")]
    picodata_path: PathBuf,
    #[builder(default)]
    format: OutputFormat,
}

impl Params {
    fn cluster_dir(&self) -> PathBuf {
        get_cluster_dir(&self.plugin_path, &self.data_dir)
    }
}

/// Output format of `status instances`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

/// State of an instance process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Stopped,
    NotStarted,
}

/// The same words as in JSON: `running`, `stopped`, `not_started`.
impl fmt::Display for ProcessState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::NotStarted => "not_started",
        };
        formatter.pad(text)
    }
}

/// What is known about one instance from `topology.toml`, its data dir,
/// `_pico_instance` and `pico.raft_status()`. Fields follow the output order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceData {
    pub tier: Option<String>,
    pub replicaset: Option<String>,
    pub name: Option<String>,
    pub raft_leader: Option<bool>,
    pub dir: String,
    pub process: ProcessState,
    pub current_state: Option<String>,
}

/// Instances of the cluster checked against `topology.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstancesReport {
    pub instances: Vec<InstanceData>,
    /// Dirs of instances not described in `topology.toml`.
    pub unexpected: Vec<String>,
    /// Why cluster data is missing while some instances are running.
    #[serde(skip)]
    pub cluster_query_error: Option<String>,
}

/// Instance dir `i<N>` found in the cluster dir.
#[derive(Debug, PartialEq, Eq)]
struct FoundInstanceDir {
    /// The instance name in Picodata.
    instance_name_from_symlink: Option<String>,
    /// `Running` or `Stopped`: the dir exists.
    process_state: ProcessState,
}

type InstanceId = u16;
type TierName<'a> = &'a str;

/// Collects the instances of the cluster started by `pike run` and checks them against
/// `topology.toml`. Prints nothing: why a running cluster gave no data goes to
/// `InstancesReport::cluster_query_error`.
///
/// # Errors
///
/// Returns an error if the cluster dir exists but cannot be read.
pub fn collect_instances(params: &Params) -> Result<InstancesReport> {
    let expected_tier_by_instance_id: BTreeMap<InstanceId, TierName> = params
        .topology
        .instance_id_ranges_by_tier()
        .flat_map(|(tier_name, instance_ids)| {
            instance_ids.map(move |instance_id| (instance_id, tier_name))
        })
        .collect();
    let cluster_dir = params.cluster_dir();
    let found_instance_dir_by_id =
        find_instance_dirs_with_symlink_names_and_process_states(&cluster_dir)?;
    let cluster_instances_query_result =
        ask_first_connectable_running_instance_for_cluster_instances(
            &cluster_dir,
            &params.picodata_path,
            &found_instance_dir_by_id,
        );

    let mut report = merge_expected_and_found_instances_into_sorted_report(
        &expected_tier_by_instance_id,
        found_instance_dir_by_id,
        cluster_instances_query_result
            .as_ref()
            .ok()
            .and_then(Option::as_ref),
    );
    report.cluster_query_error = cluster_instances_query_result
        .err()
        .map(|cluster_query_error| format!("{cluster_query_error:#}"));
    Ok(report)
}

/// Warns if the running cluster gave no data, then prints the instances in `params.format`.
///
/// # Errors
///
/// Returns an error if the cluster dir exists but cannot be read or stdout is closed.
pub fn collect_instances_and_print_them(params: &Params) -> Result<()> {
    let report = collect_instances(params)?;
    if let Some(cluster_query_error) = &report.cluster_query_error {
        log::warn!(
            "cluster data is unavailable, showing data from topology.toml and instance dirs only: {cluster_query_error}"
        );
    }

    let rendered_report = render_instances_report_in_output_format(&report, params.format)?;
    // `println!` panics on a closed pipe (`status instances | head -1`).
    let mut stdout = io::stdout().lock();
    write!(stdout, "{rendered_report}")
        .and_then(|()| stdout.flush())
        .context("failed to write instances to stdout")?;
    Ok(())
}

/// The output of `status instances` for `report`; it ends with a line break.
///
/// # Errors
///
/// Returns an error if the report cannot be serialized to JSON.
pub fn render_instances_report_in_output_format(
    report: &InstancesReport,
    output_format: OutputFormat,
) -> Result<String> {
    match output_format {
        OutputFormat::Text => Ok(render_instances_report_as_text(report)),
        OutputFormat::Json => {
            let report_json = serde_json::to_string_pretty(report)
                .context("failed to serialize instances to JSON")?;
            Ok(report_json + "\n")
        }
    }
}

/// The `pid` file outlives its instance, and after a reboot its PID may belong to another
/// process, even to an instance of another cluster: only this instance was started with
/// `--instance-dir` pointing to its dir.
#[cfg(target_os = "linux")]
fn is_pid_file_process_alive_and_started_for_instance_dir(instance_dir: &Path) -> bool {
    let Ok(pid) = read_pid_file(&instance_dir.join("pid")) else {
        return false;
    };
    // PID 0 and negative PIDs address process groups.
    if pid.as_raw() <= 0 || is_zombie(pid) {
        return false;
    }
    let Ok(canonical_instance_dir) = instance_dir.canonicalize() else {
        return false;
    };
    resolve_instance_dir_of_process_from_cmdline_and_cwd(pid)
        .is_some_and(|process_instance_dir| process_instance_dir == canonical_instance_dir)
}

/// No protection against PID reuse: the command line of a process is read from `/proc`,
/// which only Linux has.
#[cfg(not(target_os = "linux"))]
fn is_pid_file_process_alive_and_started_for_instance_dir(instance_dir: &Path) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;

    // PID 0 and negative PIDs address process groups.
    read_pid_file(&instance_dir.join("pid"))
        .is_ok_and(|pid| pid.as_raw() > 0 && kill(pid, None) != Err(Errno::ESRCH))
}

/// A missing cluster dir means that no instance has been started yet.
fn find_instance_dirs_with_symlink_names_and_process_states(
    cluster_dir: &Path,
) -> Result<BTreeMap<InstanceId, FoundInstanceDir>> {
    /// Write time and name of a symlink to an instance dir; of several such symlinks the one
    /// with the greatest rank is current.
    type SymlinkRank = (SystemTime, Reverse<String>);

    let make_cluster_dir_read_error_context =
        || format!("failed to read cluster dir {}", cluster_dir.display());
    let cluster_dir_entries = match fs::read_dir(cluster_dir) {
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        cluster_dir_entries => {
            cluster_dir_entries.with_context(make_cluster_dir_read_error_context)?
        }
    };

    let mut found_instance_dir_by_id = BTreeMap::new();
    // `read_dir` may list a symlink before its target, so names are attached after the scan.
    let mut newest_symlink_by_instance_id: BTreeMap<InstanceId, SymlinkRank> = BTreeMap::new();
    for cluster_dir_entry in cluster_dir_entries {
        let cluster_dir_entry =
            cluster_dir_entry.with_context(make_cluster_dir_read_error_context)?;
        // `pike run` re-creates symlinks while instances start: an entry listed by `read_dir`
        // may be gone already, which is the same as a missing symlink.
        let entry_file_type = match cluster_dir_entry.file_type() {
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            entry_file_type => entry_file_type.with_context(make_cluster_dir_read_error_context)?,
        };
        // `pike run` makes only UTF-8 names.
        let Ok(entry_name) = cluster_dir_entry.file_name().into_string() else {
            continue;
        };

        if entry_file_type.is_dir() {
            if let Some(instance_id) = PicodataInstance::parse_id_from_dir_name(&entry_name) {
                let process_state = if is_pid_file_process_alive_and_started_for_instance_dir(
                    &cluster_dir_entry.path(),
                ) {
                    ProcessState::Running
                } else {
                    ProcessState::Stopped
                };
                let found_instance_dir = FoundInstanceDir {
                    instance_name_from_symlink: None,
                    process_state,
                };
                found_instance_dir_by_id.insert(instance_id, found_instance_dir);
            }
        } else if entry_file_type.is_symlink() {
            let Some((symlink_target, symlink_write_time)) =
                read_symlink_target_and_write_time_unless_vanished(&cluster_dir_entry.path())
                    .with_context(make_cluster_dir_read_error_context)?
            else {
                continue;
            };
            let Some(instance_id) = symlink_target
                .file_name()
                .and_then(OsStr::to_str)
                .and_then(PicodataInstance::parse_id_from_dir_name)
            else {
                continue;
            };
            // `pike run` re-creates only the symlink of the current instance name, so of several
            // symlinks to one dir the newest is current; ties go to the lexicographically first
            // name, which does not depend on the order of `read_dir`.
            let symlink_rank = (symlink_write_time, Reverse(entry_name));
            match newest_symlink_by_instance_id.entry(instance_id) {
                Entry::Vacant(vacant_entry) => {
                    vacant_entry.insert(symlink_rank);
                }
                Entry::Occupied(mut occupied_entry) if symlink_rank > *occupied_entry.get() => {
                    occupied_entry.insert(symlink_rank);
                }
                Entry::Occupied(_) => {}
            }
        }
    }

    for (instance_id, (_, Reverse(symlink_name))) in newest_symlink_by_instance_id {
        if let Some(found_instance_dir) = found_instance_dir_by_id.get_mut(&instance_id) {
            found_instance_dir.instance_name_from_symlink = Some(symlink_name);
        }
    }
    Ok(found_instance_dir_by_id)
}

/// `None` if the symlink is gone after `read_dir` listed it.
fn read_symlink_target_and_write_time_unless_vanished(
    symlink_path: &Path,
) -> io::Result<Option<(PathBuf, SystemTime)>> {
    let read_target_and_write_time = || -> io::Result<(PathBuf, SystemTime)> {
        let symlink_target = fs::read_link(symlink_path)?;
        let symlink_write_time = fs::symlink_metadata(symlink_path)?.modified()?;
        Ok((symlink_target, symlink_write_time))
    };
    match read_target_and_write_time() {
        Ok(symlink_target_and_write_time) => Ok(Some(symlink_target_and_write_time)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Asks the first running instance (by `N`) that accepts connections on `admin.sock`.
/// `Ok(None)` if no instance is running: there is nobody to ask, and that is not a problem;
/// `Err` holds why a running cluster gave no data.
fn ask_first_connectable_running_instance_for_cluster_instances(
    cluster_dir: &Path,
    picodata_path: &PathBuf,
    found_instance_dir_by_id: &BTreeMap<InstanceId, FoundInstanceDir>,
) -> Result<Option<ClusterInstances>> {
    const CLUSTER_INSTANCES_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

    let mut running_instance_dir_names = found_instance_dir_by_id
        .iter()
        .filter(|(_, found_instance_dir)| found_instance_dir.process_state == ProcessState::Running)
        .map(|(&instance_id, _)| PicodataInstance::make_name(instance_id))
        .peekable();
    if running_instance_dir_names.peek().is_none() {
        return Ok(None);
    }

    let Some(connectable_instance_dir_name) =
        running_instance_dir_names.find(|running_instance_dir_name| {
            get_active_socket_path(cluster_dir, running_instance_dir_name).is_some()
        })
    else {
        bail!("no running instance accepts connections on admin.sock");
    };

    InstanceSocketClient::new(
        &cluster_dir.join(connectable_instance_dir_name),
        picodata_path,
    )
    .with_query_timeout(CLUSTER_INSTANCES_QUERY_TIMEOUT)
    .cluster_instances()
    .map(Some)
}

/// An expected instance without a dir is `not_started`; a dir missing from `topology.toml`
/// is also listed in `unexpected`. `cluster_query_error` is left for the caller.
fn merge_expected_and_found_instances_into_sorted_report(
    expected_tier_by_instance_id: &BTreeMap<InstanceId, TierName<'_>>,
    mut found_instance_dir_by_id: BTreeMap<InstanceId, FoundInstanceDir>,
    cluster_instances: Option<&ClusterInstances>,
) -> InstancesReport {
    let unexpected = found_instance_dir_by_id
        .keys()
        .filter(|instance_id| !expected_tier_by_instance_id.contains_key(instance_id))
        .map(|&instance_id| PicodataInstance::make_name(instance_id))
        .collect();

    let make_instance_data_from_found_dir_and_cluster_row =
        |instance_id: InstanceId, found_instance_dir: Option<FoundInstanceDir>| {
            let (instance_name, process_state) =
                found_instance_dir.map_or((None, ProcessState::NotStarted), |found_instance_dir| {
                    (
                        found_instance_dir.instance_name_from_symlink,
                        found_instance_dir.process_state,
                    )
                });
            let pico_instance_row = instance_name.as_deref().and_then(|instance_name| {
                cluster_instances?
                    .instances
                    .iter()
                    .find(|pico_instance_row| pico_instance_row.name == instance_name)
            });
            // Leader id 0: raft has no leader now.
            let is_raft_leader = cluster_instances
                .filter(|cluster_instances| cluster_instances.raft_leader_id != 0)
                .map(|cluster_instances| {
                    pico_instance_row.is_some_and(|pico_instance_row| {
                        pico_instance_row.raft_id == cluster_instances.raft_leader_id
                    })
                });
            let instance_data = InstanceData {
                tier: pico_instance_row
                    .map(|pico_instance_row| pico_instance_row.tier.clone())
                    .or_else(|| {
                        expected_tier_by_instance_id
                            .get(&instance_id)
                            .map(|&expected_tier| expected_tier.to_owned())
                    }),
                replicaset: pico_instance_row
                    .map(|pico_instance_row| pico_instance_row.replicaset_name.clone()),
                name: instance_name,
                raft_leader: is_raft_leader,
                dir: PicodataInstance::make_name(instance_id),
                process: process_state,
                current_state: pico_instance_row
                    .map(|pico_instance_row| pico_instance_row.current_state.clone()),
            };
            (instance_id, instance_data)
        };

    let expected_and_found_instance_ids: BTreeSet<InstanceId> = expected_tier_by_instance_id
        .keys()
        .chain(found_instance_dir_by_id.keys())
        .copied()
        .collect();
    let mut instance_data_with_ids: Vec<(InstanceId, InstanceData)> =
        expected_and_found_instance_ids
            .into_iter()
            .map(|instance_id| {
                make_instance_data_from_found_dir_and_cluster_row(
                    instance_id,
                    found_instance_dir_by_id.remove(&instance_id),
                )
            })
            .collect();
    sort_instances_by_tier_replicaset_rank_and_id(&mut instance_data_with_ids);

    InstancesReport {
        instances: instance_data_with_ids
            .into_iter()
            .map(|(_, instance_data)| instance_data)
            .collect(),
        unexpected,
        cluster_query_error: None,
    }
}

/// The smallest `InstanceId` among the instances of a replicaset: replicasets go in the order
/// `pike run` started them, so `default_2` goes before `default_10`.
type ReplicasetRank = InstanceId;

/// The rank of a replicaset is the smallest instance id in it. A missing tier goes to the end
/// of the list, a missing replicaset goes to the end of its tier.
fn sort_instances_by_tier_replicaset_rank_and_id(
    instance_data_with_ids: &mut [(InstanceId, InstanceData)],
) {
    type ReplicasetName<'a> = &'a str;
    type TierAndReplicasetNames<'a> = (Option<TierName<'a>>, ReplicasetName<'a>);

    let mut replicaset_rank_by_tier_and_replicaset =
        HashMap::<TierAndReplicasetNames, ReplicasetRank>::new();
    for (instance_id, instance_data) in &*instance_data_with_ids {
        if let Some(replicaset) = instance_data.replicaset.as_deref() {
            replicaset_rank_by_tier_and_replicaset
                .entry((instance_data.tier.as_deref(), replicaset))
                .and_modify(|replicaset_rank| {
                    *replicaset_rank = (*replicaset_rank).min(*instance_id);
                })
                .or_insert(*instance_id);
        }
    }
    // The map above borrows `instance_data_with_ids`, which `sort_by` needs mutably.
    let replicaset_rank_by_instance_id: HashMap<InstanceId, ReplicasetRank> =
        instance_data_with_ids
            .iter()
            .filter_map(|(instance_id, instance_data)| {
                let replicaset = instance_data.replicaset.as_deref()?;
                Some((
                    *instance_id,
                    replicaset_rank_by_tier_and_replicaset
                        [&(instance_data.tier.as_deref(), replicaset)],
                ))
            })
            .collect();

    instance_data_with_ids.sort_by(|left_instance, right_instance| {
        make_instance_sort_key(left_instance, &replicaset_rank_by_instance_id).cmp(
            &make_instance_sort_key(right_instance, &replicaset_rank_by_instance_id),
        )
    });
}

/// `bool` flags put a missing tier and a missing replicaset rank after the present ones.
type InstanceSortKey<'a> = (
    bool,
    Option<TierName<'a>>,
    bool,
    Option<ReplicasetRank>,
    InstanceId,
);

fn make_instance_sort_key<'a>(
    (instance_id, instance_data): &'a (InstanceId, InstanceData),
    replicaset_rank_by_instance_id: &HashMap<InstanceId, ReplicasetRank>,
) -> InstanceSortKey<'a> {
    let replicaset_rank = replicaset_rank_by_instance_id.get(instance_id).copied();
    (
        instance_data.tier.is_none(),
        instance_data.tier.as_deref(),
        replicaset_rank.is_none(),
        replicaset_rank,
        *instance_id,
    )
}

/// In the order of `InstanceData` fields.
const TEXT_TABLE_COLUMN_HEADERS: [&str; 6] =
    ["TIER", "REPLICASET", "INSTANCE", "DIR", "PROCESS", "STATE"];

/// An aligned table, an empty line, the summary line and the legend line.
fn render_instances_report_as_text(report: &InstancesReport) -> String {
    InstancesReportText(report).to_string()
}

struct InstancesReportText<'a>(&'a InstancesReport);

impl fmt::Display for InstancesReportText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const RAFT_LEADER_LEGEND_LINE: &str = "★ - raft leader";

        self.write_instances_table(formatter)?;
        writeln!(formatter)?;
        self.write_summary_line(formatter)?;
        writeln!(formatter, "{RAFT_LEADER_LEGEND_LINE}")
    }
}

impl InstancesReportText<'_> {
    /// Every cell but the last one is padded to the width of its column, so lines have no
    /// trailing spaces.
    fn write_instances_table(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        const TEXT_TABLE_COLUMN_SEPARATOR: &str = "  ";

        let header_row_cells = TEXT_TABLE_COLUMN_HEADERS.map(str::to_owned);
        let table_rows: Vec<[String; TEXT_TABLE_COLUMN_HEADERS.len()]> =
            iter::once(header_row_cells)
                .chain(self.0.instances.iter().map(make_text_table_row_cells))
                .collect();

        // In chars, the same way `format!` pads: the leader prefix counts as one.
        let mut column_widths = [0; TEXT_TABLE_COLUMN_HEADERS.len()];
        for row_cells in &table_rows {
            for (column_width, cell) in column_widths.iter_mut().zip(row_cells) {
                *column_width = (*column_width).max(cell.chars().count());
            }
        }

        for row_cells in &table_rows {
            let [padded_cells @ .., last_cell] = row_cells;
            for (cell, &column_width) in padded_cells.iter().zip(&column_widths) {
                write!(
                    formatter,
                    "{cell:<column_width$}{TEXT_TABLE_COLUMN_SEPARATOR}"
                )?;
            }
            writeln!(formatter, "{last_cell}")?;
        }
        Ok(())
    }

    /// `{running}/{total} instances running, {not_started} not started`, then
    /// `, raft leader unknown` if no instance is the leader and the unexpected dirs, if any.
    fn write_summary_line(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let report = self.0;
        let count_instances_in_process_state = |process_state: ProcessState| {
            report
                .instances
                .iter()
                .filter(|instance_data| instance_data.process == process_state)
                .count()
        };
        let running_count = count_instances_in_process_state(ProcessState::Running);
        let not_started_count = count_instances_in_process_state(ProcessState::NotStarted);
        let total_count = report.instances.len();

        write!(
            formatter,
            "{running_count}/{total_count} instances running, {not_started_count} not started"
        )?;
        if !report
            .instances
            .iter()
            .any(|instance_data| instance_data.raft_leader == Some(true))
        {
            formatter.write_str(", raft leader unknown")?;
        }
        if !report.unexpected.is_empty() {
            write!(
                formatter,
                ", {} not described in topology.toml: {}",
                report.unexpected.len(),
                report.unexpected.join(", ")
            )?;
        }
        writeln!(formatter)
    }
}

fn make_text_table_row_cells(
    instance_data: &InstanceData,
) -> [String; TEXT_TABLE_COLUMN_HEADERS.len()] {
    const TEXT_TABLE_MISSING_VALUE: &str = "-";
    const RAFT_LEADER_NAME_PREFIX: &str = "★ ";
    /// As wide as `RAFT_LEADER_NAME_PREFIX`, so instance names stay aligned.
    const NOT_RAFT_LEADER_NAME_PREFIX: &str = "  ";

    let value_or_missing_cell = |value: &Option<String>| {
        value
            .as_deref()
            .unwrap_or(TEXT_TABLE_MISSING_VALUE)
            .to_owned()
    };
    let instance_name_prefix = if instance_data.raft_leader == Some(true) {
        RAFT_LEADER_NAME_PREFIX
    } else {
        NOT_RAFT_LEADER_NAME_PREFIX
    };
    [
        value_or_missing_cell(&instance_data.tier),
        value_or_missing_cell(&instance_data.replicaset),
        format!(
            "{instance_name_prefix}{}",
            value_or_missing_cell(&instance_data.name)
        ),
        instance_data.dir.clone(),
        instance_data.process.to_string(),
        value_or_missing_cell(&instance_data.current_state),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::lib::instance_info::PicoInstanceRow;
    #[cfg(target_os = "linux")]
    use crate::commands::lib::PICODATA_INSTANCE_DIR_OPTION;
    #[cfg(target_os = "linux")]
    use crate::commands::test_helpers::spawn_fake_picodata_instance_and_wait_until_started;
    use crate::commands::test_helpers::{
        make_topology_with_tier_sizes, UniqueTempDirRemovedOnDrop,
    };
    use std::os::unix::fs::symlink;
    use std::process::Command;

    /// Returns the guard of the cluster dir and its `i1`; the `pid` file is written only for
    /// `Some`.
    fn create_temp_cluster_dir_with_instance_dir_i1(
        prefix: &str,
        pid_file_contents: Option<&str>,
    ) -> (UniqueTempDirRemovedOnDrop, PathBuf) {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create(prefix);
        let instance_dir = cluster_dir.path().join("i1");
        fs::create_dir(&instance_dir).unwrap();
        if let Some(pid_file_contents) = pid_file_contents {
            fs::write(instance_dir.join("pid"), pid_file_contents).unwrap();
        }
        (cluster_dir, instance_dir)
    }

    /// The `pid` file of `<cluster dir>/i1` names a fake instance started in `<cluster dir>`.
    #[cfg(target_os = "linux")]
    fn is_instance_running_when_fake_picodata_instance_started_with_instance_dir_arg(
        prefix: &str,
        instance_dir_arg: &str,
    ) -> bool {
        let (cluster_dir, instance_dir) =
            create_temp_cluster_dir_with_instance_dir_i1(prefix, None);
        fs::create_dir(cluster_dir.path().join("i2")).unwrap();
        let mut child = spawn_fake_picodata_instance_and_wait_until_started(
            cluster_dir.path(),
            &[PICODATA_INSTANCE_DIR_OPTION, instance_dir_arg],
        );
        fs::write(instance_dir.join("pid"), child.id().to_string()).unwrap();

        let is_running = is_pid_file_process_alive_and_started_for_instance_dir(&instance_dir);
        child.kill().unwrap();
        child.wait().unwrap();
        is_running
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_running_when_started_with_this_instance_dir() {
        assert!(
            is_instance_running_when_fake_picodata_instance_started_with_instance_dir_arg(
                "own-dir", "././i1"
            )
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_not_running_when_started_with_other_instance_dir() {
        assert!(
            !is_instance_running_when_fake_picodata_instance_started_with_instance_dir_arg(
                "other-dir",
                "././i2"
            )
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn process_running_for_current_process() {
        let (_cluster_dir, instance_dir) = create_temp_cluster_dir_with_instance_dir_i1(
            "running",
            Some(&std::process::id().to_string()),
        );

        assert!(is_pid_file_process_alive_and_started_for_instance_dir(
            &instance_dir
        ));
    }

    #[test]
    fn process_not_running_without_pid_file() {
        let (_cluster_dir, instance_dir) =
            create_temp_cluster_dir_with_instance_dir_i1("no-pid", None);

        assert!(!is_pid_file_process_alive_and_started_for_instance_dir(
            &instance_dir
        ));
    }

    #[test]
    fn process_not_running_with_invalid_pid() {
        let (_cluster_dir, instance_dir) =
            create_temp_cluster_dir_with_instance_dir_i1("invalid-pid", None);

        for pid_file_contents in ["abc", "0", "-1"] {
            fs::write(instance_dir.join("pid"), pid_file_contents).unwrap();
            assert!(
                !is_pid_file_process_alive_and_started_for_instance_dir(&instance_dir),
                "{pid_file_contents}"
            );
        }
    }

    #[test]
    fn process_not_running_after_exit() {
        let mut child = Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        let (_cluster_dir, instance_dir) =
            create_temp_cluster_dir_with_instance_dir_i1("exited", Some(&child.id().to_string()));

        assert!(!is_pid_file_process_alive_and_started_for_instance_dir(
            &instance_dir
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_not_running_for_zombie() {
        use crate::commands::lib::is_zombie;
        use nix::unistd::Pid;
        use std::time::{Duration, Instant};

        let mut child = Command::new("true").spawn().unwrap();
        let child_pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !is_zombie(child_pid) {
            assert!(Instant::now() < deadline, "child {child_pid} did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        let (_cluster_dir, instance_dir) =
            create_temp_cluster_dir_with_instance_dir_i1("zombie", Some(&child_pid.to_string()));

        assert!(!is_pid_file_process_alive_and_started_for_instance_dir(
            &instance_dir
        ));
        child.wait().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn process_not_running_for_reused_pid() {
        // The test process got the PID of a gone instance; it has no `--instance-dir`.
        let (_cluster_dir, instance_dir) = create_temp_cluster_dir_with_instance_dir_i1(
            "reused-pid",
            Some(&std::process::id().to_string()),
        );

        assert!(!is_pid_file_process_alive_and_started_for_instance_dir(
            &instance_dir
        ));
    }

    #[test]
    fn find_instance_dirs_reads_dirs_and_symlink_names() {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create("scan");
        for dir_name in ["i1", "i2", "i01", "plugins"] {
            fs::create_dir(cluster_dir.path().join(dir_name)).unwrap();
        }
        for (symlink_name, symlink_target) in [
            ("default_1_1", "i1"),
            ("b_name", "i2"),
            ("a_name", "i2"),
            ("ghost", "i9"),
        ] {
            symlink(symlink_target, cluster_dir.path().join(symlink_name)).unwrap();
        }
        fs::write(cluster_dir.path().join("notes.txt"), "").unwrap();

        let found_instance_dir_by_id =
            find_instance_dirs_with_symlink_names_and_process_states(cluster_dir.path()).unwrap();

        assert_eq!(
            found_instance_dir_by_id,
            make_found_instance_dir_by_id(&[
                (1, Some("default_1_1"), ProcessState::Stopped),
                (2, Some("a_name"), ProcessState::Stopped),
            ])
        );
    }

    #[test]
    fn find_instance_dirs_prefers_newest_of_several_symlinks() {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create("newest-symlink");
        fs::create_dir(cluster_dir.path().join("i1")).unwrap();
        // The stale name sorts first, so only the write time can pick the current one.
        symlink("i1", cluster_dir.path().join("a_stale_name")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        symlink("i1", cluster_dir.path().join("b_current_name")).unwrap();

        let found_instance_dir_by_id =
            find_instance_dirs_with_symlink_names_and_process_states(cluster_dir.path()).unwrap();

        assert_eq!(
            found_instance_dir_by_id,
            make_found_instance_dir_by_id(&[(1, Some("b_current_name"), ProcessState::Stopped)])
        );
    }

    #[test]
    fn read_symlink_target_of_vanished_symlink_is_none() {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create("vanished-symlink");
        let symlink_path = cluster_dir.path().join("default_1_1");
        symlink("i1", &symlink_path).unwrap();

        let (symlink_target, _) = read_symlink_target_and_write_time_unless_vanished(&symlink_path)
            .unwrap()
            .unwrap();
        assert_eq!(symlink_target, Path::new("i1"));
        fs::remove_file(&symlink_path).unwrap();
        assert!(
            read_symlink_target_and_write_time_unless_vanished(&symlink_path)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn find_instance_dirs_without_cluster_dir_is_empty() {
        let temp_dir = UniqueTempDirRemovedOnDrop::create("missing-cluster-dir");

        assert_eq!(
            find_instance_dirs_with_symlink_names_and_process_states(
                &temp_dir.path().join("cluster")
            )
            .unwrap(),
            BTreeMap::new()
        );
    }

    fn make_expected_tier_by_instance_id() -> BTreeMap<InstanceId, TierName<'static>> {
        BTreeMap::from([(1, "default"), (2, "default")])
    }

    fn make_found_instance_dir_by_id(
        found_instance_dirs: &[(InstanceId, Option<&str>, ProcessState)],
    ) -> BTreeMap<InstanceId, FoundInstanceDir> {
        found_instance_dirs
            .iter()
            .map(
                |&(instance_id, instance_name_from_symlink, process_state)| {
                    let found_instance_dir = FoundInstanceDir {
                        instance_name_from_symlink: instance_name_from_symlink.map(str::to_owned),
                        process_state,
                    };
                    (instance_id, found_instance_dir)
                },
            )
            .collect()
    }

    fn make_pico_instance_row(
        name: &str,
        tier: &str,
        replicaset_name: &str,
        raft_id: u64,
    ) -> PicoInstanceRow {
        PicoInstanceRow {
            name: name.to_owned(),
            tier: tier.to_owned(),
            replicaset_name: replicaset_name.to_owned(),
            current_state: "Online".to_owned(),
            raft_id,
        }
    }

    fn make_default_1_replicaset_cluster_instances(raft_leader_id: u64) -> ClusterInstances {
        ClusterInstances {
            raft_leader_id,
            instances: vec![
                make_pico_instance_row("default_1_1", "default", "default_1", 1),
                make_pico_instance_row("default_1_2", "default", "default_1", 2),
            ],
        }
    }

    /// A base for `InstanceData { ..., ..make_not_started_default_tier_instance_data(dir) }`.
    fn make_not_started_default_tier_instance_data(dir: &str) -> InstanceData {
        InstanceData {
            tier: Some("default".to_owned()),
            replicaset: None,
            name: None,
            raft_leader: None,
            dir: dir.to_owned(),
            process: ProcessState::NotStarted,
            current_state: None,
        }
    }

    fn find_instance_data_by_dir<'a>(report: &'a InstancesReport, dir: &str) -> &'a InstanceData {
        report
            .instances
            .iter()
            .find(|instance_data| instance_data.dir == dir)
            .unwrap_or_else(|| panic!("no {dir} in {report:?}"))
    }

    #[test]
    fn merge_fills_cluster_fields_and_leader() {
        let found_instance_dir_by_id = make_found_instance_dir_by_id(&[
            (1, Some("default_1_1"), ProcessState::Running),
            (2, Some("default_1_2"), ProcessState::Running),
        ]);
        let cluster_instances = make_default_1_replicaset_cluster_instances(2);

        let report = merge_expected_and_found_instances_into_sorted_report(
            &make_expected_tier_by_instance_id(),
            found_instance_dir_by_id,
            Some(&cluster_instances),
        );

        let make_running_default_1_instance_data =
            |name: &str, dir: &str, is_raft_leader: bool| InstanceData {
                replicaset: Some("default_1".to_owned()),
                name: Some(name.to_owned()),
                raft_leader: Some(is_raft_leader),
                process: ProcessState::Running,
                current_state: Some("Online".to_owned()),
                ..make_not_started_default_tier_instance_data(dir)
            };
        assert_eq!(
            report,
            InstancesReport {
                instances: vec![
                    make_running_default_1_instance_data("default_1_1", "i1", false),
                    make_running_default_1_instance_data("default_1_2", "i2", true),
                ],
                unexpected: vec![],
                cluster_query_error: None,
            }
        );
    }

    #[test]
    fn merge_marks_missing_expected_as_not_started() {
        let found_instance_dir_by_id =
            make_found_instance_dir_by_id(&[(1, Some("default_1_1"), ProcessState::Running)]);
        let cluster_instances = make_default_1_replicaset_cluster_instances(1);

        let report = merge_expected_and_found_instances_into_sorted_report(
            &make_expected_tier_by_instance_id(),
            found_instance_dir_by_id,
            Some(&cluster_instances),
        );

        assert_eq!(
            find_instance_data_by_dir(&report, "i2"),
            &InstanceData {
                raft_leader: Some(false),
                ..make_not_started_default_tier_instance_data("i2")
            }
        );
        assert_eq!(
            find_instance_data_by_dir(&report, "i1").raft_leader,
            Some(true)
        );
    }

    #[test]
    fn merge_lists_unexpected_dirs() {
        let found_instance_dir_by_id = make_found_instance_dir_by_id(&[
            (1, Some("default_1_1"), ProcessState::Running),
            (2, Some("default_1_2"), ProcessState::Running),
            (3, Some("default_2_1"), ProcessState::Running),
            (5, Some("stray"), ProcessState::Stopped),
        ]);
        let mut cluster_instances = make_default_1_replicaset_cluster_instances(1);
        cluster_instances.instances.push(make_pico_instance_row(
            "default_2_1",
            "default",
            "default_2",
            3,
        ));

        let report = merge_expected_and_found_instances_into_sorted_report(
            &make_expected_tier_by_instance_id(),
            found_instance_dir_by_id,
            Some(&cluster_instances),
        );

        assert_eq!(report.unexpected, ["i3", "i5"]);
        assert_eq!(
            find_instance_data_by_dir(&report, "i3").tier.as_deref(),
            Some("default")
        );
        assert_eq!(find_instance_data_by_dir(&report, "i5").tier, None);
        assert_eq!(report.instances.len(), 4);
    }

    #[test]
    fn merge_without_cluster_uses_topology() {
        let found_instance_dir_by_id = make_found_instance_dir_by_id(&[
            (1, Some("default_1_1"), ProcessState::Stopped),
            (2, Some("default_1_2"), ProcessState::Stopped),
        ]);

        let report = merge_expected_and_found_instances_into_sorted_report(
            &make_expected_tier_by_instance_id(),
            found_instance_dir_by_id,
            None,
        );

        assert_eq!(
            find_instance_data_by_dir(&report, "i1"),
            &InstanceData {
                name: Some("default_1_1".to_owned()),
                process: ProcessState::Stopped,
                ..make_not_started_default_tier_instance_data("i1")
            }
        );
        assert_eq!(
            find_instance_data_by_dir(&report, "i2").name.as_deref(),
            Some("default_1_2")
        );
    }

    #[test]
    fn merge_unknown_leader_is_null() {
        let found_instance_dir_by_id = make_found_instance_dir_by_id(&[
            (1, Some("default_1_1"), ProcessState::Running),
            (2, Some("default_1_2"), ProcessState::Running),
        ]);
        let cluster_instances = make_default_1_replicaset_cluster_instances(0);

        let report = merge_expected_and_found_instances_into_sorted_report(
            &make_expected_tier_by_instance_id(),
            found_instance_dir_by_id,
            Some(&cluster_instances),
        );

        assert!(report
            .instances
            .iter()
            .all(|instance_data| instance_data.raft_leader.is_none()));
    }

    #[test]
    fn merge_found_dir_without_symlink_uses_topology_tier() {
        let found_instance_dir_by_id = make_found_instance_dir_by_id(&[
            (1, Some("default_1_1"), ProcessState::Running),
            (2, None, ProcessState::Running),
        ]);
        let cluster_instances = make_default_1_replicaset_cluster_instances(1);

        let report = merge_expected_and_found_instances_into_sorted_report(
            &make_expected_tier_by_instance_id(),
            found_instance_dir_by_id,
            Some(&cluster_instances),
        );

        assert_eq!(
            find_instance_data_by_dir(&report, "i2"),
            &InstanceData {
                raft_leader: Some(false),
                process: ProcessState::Running,
                ..make_not_started_default_tier_instance_data("i2")
            }
        );
    }

    fn make_instance_data_with_id_for_sorting(
        instance_id: InstanceId,
        tier: Option<&str>,
        replicaset: Option<&str>,
    ) -> (InstanceId, InstanceData) {
        let instance_data = InstanceData {
            tier: tier.map(str::to_owned),
            replicaset: replicaset.map(str::to_owned),
            ..make_not_started_default_tier_instance_data(&PicodataInstance::make_name(instance_id))
        };
        (instance_id, instance_data)
    }

    #[test]
    fn sort_instances_orders_by_tier_replicaset_rank_and_id() {
        let mut instance_data_with_ids = [
            make_instance_data_with_id_for_sorting(7, Some("a"), Some("a_2")),
            make_instance_data_with_id_for_sorting(6, None, None),
            make_instance_data_with_id_for_sorting(1, Some("b"), Some("b_1")),
            make_instance_data_with_id_for_sorting(5, Some("a"), None),
            make_instance_data_with_id_for_sorting(4, Some("a"), Some("a_10")),
            make_instance_data_with_id_for_sorting(3, Some("a"), Some("a_2")),
            make_instance_data_with_id_for_sorting(2, Some("a"), Some("a_10")),
        ];

        sort_instances_by_tier_replicaset_rank_and_id(&mut instance_data_with_ids);

        let sorted_instance_ids: Vec<InstanceId> = instance_data_with_ids
            .iter()
            .map(|(instance_id, _)| *instance_id)
            .collect();
        assert_eq!(sorted_instance_ids, [2, 4, 3, 7, 5, 1, 6]);
    }

    #[test]
    fn ask_cluster_without_running_instances_is_none() {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create("query-stopped");
        let found_instance_dir_by_id =
            make_found_instance_dir_by_id(&[(1, Some("default_1_1"), ProcessState::Stopped)]);

        assert!(matches!(
            ask_first_connectable_running_instance_for_cluster_instances(
                cluster_dir.path(),
                &PathBuf::from("picodata"),
                &found_instance_dir_by_id
            ),
            Ok(None)
        ));
    }

    #[test]
    fn ask_cluster_without_connectable_socket_reports_reason() {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create("query-no-socket");
        let found_instance_dir_by_id =
            make_found_instance_dir_by_id(&[(1, Some("default_1_1"), ProcessState::Running)]);

        let cluster_query_error = ask_first_connectable_running_instance_for_cluster_instances(
            cluster_dir.path(),
            &PathBuf::from("picodata"),
            &found_instance_dir_by_id,
        )
        .unwrap_err();

        assert_eq!(
            cluster_query_error.to_string(),
            "no running instance accepts connections on admin.sock"
        );
    }

    #[test]
    fn collect_instances_without_cluster_dir_lists_topology_instances_of_every_tier() {
        let plugin_dir = UniqueTempDirRemovedOnDrop::create("collect-empty");
        // Numbered in the order of tier names, as `pike run` does: `a` gets `i1..i2`.
        let params = ParamsBuilder::default()
            .topology(make_topology_with_tier_sizes(&[("b", 1, 1), ("a", 1, 2)]))
            .plugin_path(plugin_dir.path().to_path_buf())
            .build()
            .unwrap();

        let report = collect_instances(&params).unwrap();

        let dir_tier_and_process_state: Vec<(&str, Option<&str>, ProcessState)> = report
            .instances
            .iter()
            .map(|instance_data| {
                (
                    instance_data.dir.as_str(),
                    instance_data.tier.as_deref(),
                    instance_data.process,
                )
            })
            .collect();
        assert_eq!(
            dir_tier_and_process_state,
            [
                ("i1", Some("a"), ProcessState::NotStarted),
                ("i2", Some("a"), ProcessState::NotStarted),
                ("i3", Some("b"), ProcessState::NotStarted),
            ]
        );
        assert_eq!(report.unexpected, Vec::<String>::new());
        assert_eq!(report.cluster_query_error, None);
    }

    #[test]
    fn render_text_shows_leader_stopped_not_started_and_unexpected_instances() {
        let make_default_tier_instance_data_in_cluster =
            |dir: &str,
             replicaset: &str,
             name: &str,
             process: ProcessState,
             current_state: &str| {
                InstanceData {
                    replicaset: Some(replicaset.to_owned()),
                    name: Some(name.to_owned()),
                    raft_leader: Some(false),
                    process,
                    current_state: Some(current_state.to_owned()),
                    ..make_not_started_default_tier_instance_data(dir)
                }
            };
        let report = InstancesReport {
            instances: vec![
                InstanceData {
                    raft_leader: Some(true),
                    ..make_default_tier_instance_data_in_cluster(
                        "i1",
                        "default_1",
                        "default_1_1",
                        ProcessState::Running,
                        "Online",
                    )
                },
                make_default_tier_instance_data_in_cluster(
                    "i2",
                    "default_1",
                    "default_1_2",
                    ProcessState::Running,
                    "Online",
                ),
                make_default_tier_instance_data_in_cluster(
                    "i3",
                    "default_2",
                    "default_2_1",
                    ProcessState::Stopped,
                    "Offline",
                ),
                make_default_tier_instance_data_in_cluster(
                    "i5",
                    "default_3",
                    "default_3_1",
                    ProcessState::Running,
                    "Online",
                ),
                InstanceData {
                    raft_leader: Some(false),
                    ..make_not_started_default_tier_instance_data("i4")
                },
            ],
            unexpected: vec!["i5".to_owned()],
            cluster_query_error: None,
        };

        assert_eq!(
            render_instances_report_as_text(&report),
            "\
TIER     REPLICASET  INSTANCE       DIR  PROCESS      STATE
default  default_1   ★ default_1_1  i1   running      Online
default  default_1     default_1_2  i2   running      Online
default  default_2     default_2_1  i3   stopped      Offline
default  default_3     default_3_1  i5   running      Online
default  -             -            i4   not_started  -

3/5 instances running, 1 not started, 1 not described in topology.toml: i5
★ - raft leader
"
        );
    }

    #[test]
    fn render_text_reports_unknown_raft_leader() {
        let report = InstancesReport {
            instances: [
                ("default_1_1", "i1"),
                ("default_1_2", "i2"),
                ("default_2_1", "i3"),
                ("default_2_2", "i4"),
            ]
            .into_iter()
            .map(|(name, dir)| InstanceData {
                name: Some(name.to_owned()),
                process: ProcessState::Stopped,
                ..make_not_started_default_tier_instance_data(dir)
            })
            .collect(),
            unexpected: vec![],
            cluster_query_error: None,
        };

        assert_eq!(
            render_instances_report_as_text(&report),
            "\
TIER     REPLICASET  INSTANCE       DIR  PROCESS  STATE
default  -             default_1_1  i1   stopped  -
default  -             default_1_2  i2   stopped  -
default  -             default_2_1  i3   stopped  -
default  -             default_2_2  i4   stopped  -

0/4 instances running, 0 not started, raft leader unknown
★ - raft leader
"
        );
    }

    #[test]
    fn json_keeps_field_order_and_skips_query_error() {
        let report = InstancesReport {
            instances: vec![InstanceData {
                raft_leader: Some(false),
                ..make_not_started_default_tier_instance_data("i4")
            }],
            unexpected: vec![],
            cluster_query_error: Some("x".to_owned()),
        };

        assert_eq!(
            serde_json::to_string_pretty(&report).unwrap(),
            r#"{
  "instances": [
    {
      "tier": "default",
      "replicaset": null,
      "name": null,
      "raft_leader": false,
      "dir": "i4",
      "process": "not_started",
      "current_state": null
    }
  ],
  "unexpected": []
}"#
        );
    }

    #[test]
    fn process_state_display_matches_json() {
        for process_state in [
            ProcessState::Running,
            ProcessState::Stopped,
            ProcessState::NotStarted,
        ] {
            assert_eq!(
                format!("\"{process_state}\""),
                serde_json::to_string(&process_state).unwrap()
            );
        }
    }
}
