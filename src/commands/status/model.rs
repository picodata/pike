use super::collector::DirectoryInstance;
use crate::commands::lib::instance_info::{ClusterInstance, InstanceState};
use crate::commands::run::PicodataInstance;
use serde::Serialize;
use std::fmt::{self, Display, Formatter};

/// Result of the `status instances` command.
#[derive(Debug, Serialize)]
pub struct InstancesStatus {
    /// The instance list, sorted when collected:
    /// - tiers sorted alphabetically
    /// - replicasets inside tier sorted alphabetically
    /// - instances inside replicaset sorted by N (directory named `i<N>`)
    pub instances: Vec<InstanceStatus>,
    /// Directories of found instances missing from topology.toml (sorted by N)
    pub unexpected: Vec<String>,
}

/// Status of a single cluster instance
#[derive(Debug, Serialize)]
pub struct InstanceStatus {
    pub tier: Option<String>,
    pub replicaset: Option<String>,
    pub name: Option<String>,
    pub raft_leader: Option<bool>,
    pub dir: String,
    pub process: ProcessState,
    /// Instance state from picodata system table
    pub current_state: Option<InstanceState>,
}

impl InstanceStatus {
    /// Builds the status of the instance `i<n>`: the cluster fields come from
    /// the picodata system tables/functions and may be absent. In this case:
    /// - the tier falls back to the topology.toml
    /// - the replicaset and state are empty
    /// - the instance is considered as not the raft-leader
    pub(super) fn new(
        n: u16,
        found_on_disk: Option<&DirectoryInstance>,
        expected_tier: Option<&str>,
        row: Option<&ClusterInstance>,
        leader_known: bool,
    ) -> Self {
        let name = found_on_disk.and_then(|instance| instance.instance_name.clone());

        match row {
            Some(row) => Self {
                tier: Some(row.tier.clone()),
                replicaset: Some(row.replicaset_name.clone()),
                name,
                raft_leader: leader_known.then_some(row.is_leader),
                dir: PicodataInstance::make_data_dir_name(n),
                process: found_on_disk
                    .map_or(ProcessState::NotStarted, |instance| instance.process),
                current_state: Some(row.current_state),
            },
            None => Self {
                tier: expected_tier.map(str::to_string),
                replicaset: None,
                name,
                raft_leader: leader_known.then_some(false),
                dir: PicodataInstance::make_data_dir_name(n),
                process: found_on_disk
                    .map_or(ProcessState::NotStarted, |instance| instance.process),
                current_state: None,
            },
        }
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.process == ProcessState::Running
    }

    #[must_use]
    pub fn not_started(&self) -> bool {
        self.process == ProcessState::NotStarted
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.process == ProcessState::Stopped
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    /// The process is alive and not a zombie
    Running,
    /// The instance dir exists but the process is not alive or the
    /// PID file is broken (missing or unreadable)
    Stopped,
    /// The instance exists in topology.toml but its directory does not exist
    NotStarted,
}

impl Display for ProcessState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::NotStarted => "not_started",
        })
    }
}
