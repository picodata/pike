use crate::commands::lib::{find_active_socket_path, run_query_in_picodata_admin};
use anyhow::{anyhow, bail, Context, Result};
use std::{path::Path, str::FromStr};

const GET_INSTANCE_NAME: &str = "\\lua\npico.instance_info().name";
const GET_INSTANCE_CURRENT_STATE: &str = "\\lua\npico.instance_info().current_state.variant";
const GET_CLUSTER_LEADER_ID: &str = "\\lua\nbox.func[\".proc_runtime_info\"]:call().raft.leader_id";

#[derive(Clone, Copy, Debug)]
pub enum InstanceState {
    Online,
    Offline,
    Expelled,
}

impl InstanceState {
    pub fn is_online(self) -> bool {
        matches!(self, InstanceState::Online)
    }
}

impl FromStr for InstanceState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let state = match s.to_ascii_lowercase().as_str() {
            "online" => Self::Online,
            "offline" => Self::Offline,
            "expelled" => Self::Expelled,
            unknown => bail!("Unknown instane state variant: '{unknown}'"),
        };

        Ok(state)
    }
}

/// Runs input query in picodata admin.
///
/// Only single line is extracted from returned STDOUT.
fn get_lua_single_line_output(
    picodata_path: &Path,
    socket_path: &Path,
    lua_query: &str,
) -> Result<String> {
    let stdout = run_query_in_picodata_admin(picodata_path, socket_path, lua_query)?;

    let Some(output) = stdout.lines().find_map(|line| line.strip_prefix("- ")) else {
        bail!("unable to extract single line from Lua query output '{stdout}'");
    };

    Ok(output.to_string())
}

pub fn get_instance_name(picodata_path: &Path, instance_data_dir: &Path) -> Result<String> {
    let instance_socket = instance_data_dir.join("admin.sock");

    get_lua_single_line_output(picodata_path, &instance_socket, GET_INSTANCE_NAME)
}

pub fn get_instance_current_state(
    picodata_path: &Path,
    instance_data_dir: &Path,
) -> Result<InstanceState> {
    let instance_socket = instance_data_dir.join("admin.sock");

    get_lua_single_line_output(picodata_path, &instance_socket, GET_INSTANCE_CURRENT_STATE)
        .and_then(|state| state.parse())
}

pub fn get_cluster_leader_id(picodata_path: &Path, cluster_dir: &Path) -> Result<usize> {
    let Some(socket_path) = find_active_socket_path(cluster_dir)? else {
        bail!("failed to get cluster leader id information: no active socket found")
    };

    get_lua_single_line_output(picodata_path, &socket_path, GET_CLUSTER_LEADER_ID)
        .and_then(|str| str.parse().context("failed to parse leader id from string"))
        .map_err(|err| anyhow!("unable to get cluster leader id: {err}"))
}
