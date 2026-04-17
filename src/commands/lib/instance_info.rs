use crate::commands::lib::run_query_in_picodata_admin;
use anyhow::{bail, Context, Result};
use log::warn;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

const ADMIN_SOCKET_FILENAME: &str = "admin.sock";

const GET_INSTANCE_NAME: &str = "\\lua\npico.instance_info().name";
const GET_INSTANCE_CURRENT_STATE: &str = "\\lua\npico.instance_info().current_state.variant";

// Get configured number of buckets in the instance tier.
const GET_TIER_BUCKET_COUNT: &str =
    "\\lua\nbox.space._pico_tier:get{pico.whoami().tier}.bucket_count";

// Get configured number of replicasets in the instance tier.
const GET_TIER_REPLICASET_COUNT: &str =
    "\\lua\npico.sql(\"select count(*) from _pico_replicaset where tier = ?\", {pico.whoami().tier}).rows[1][1]";

// Get the UUID of the instance's replicaset.
const GET_REPLICASET_UUID: &str =
    "\\lua\npico.sql(\"select replicaset_uuid from _pico_instance where name = ?\", {pico.whoami().instance_name}).rows[1][1]";

// Get current number of buckets located on the instance
const GET_INSTANCE_BUCKET_COUNT: &str = "\\lua\nbox.space._bucket:count()";

// Get map of [replicaset_uuid, bucket_count] obtained from
// vshard router for all tiers.
const GET_VSHARD_REPLICASET_MAP: &str = "\\lua\n\
local out = {}; \
for _, router in pairs(vshard.router.internal.routers) do \
    for uuid, rs in pairs(router.replicasets) do \
        out[uuid] = rs.bucket_count; \
    end; \
end; \
return require('json').encode(out)";

fn parse_lua_json(lua_output: &str) -> Result<HashMap<String, u32>> {
    let trimmed = lua_output.trim();

    // remove wrapping single quotes if present
    let json = if trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    Ok(serde_json::from_str(json)?)
}

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

/// Client for interacting with a Picodata
/// instance over its admin socket.
pub struct InstanceSocketClient<'a> {
    socket_path: PathBuf,
    picodata_path: &'a PathBuf,
}

impl<'a> InstanceSocketClient<'a> {
    pub fn new(instance_data_dir: &Path, picodata_path: &'a PathBuf) -> Self {
        Self {
            socket_path: instance_data_dir.join(ADMIN_SOCKET_FILENAME),
            picodata_path,
        }
    }

    /// Runs input query in picodata admin.
    ///
    /// Only single line is extracted from returned STDOUT.
    fn get_lua_single_line_output(&self, lua_query: &str) -> Result<String> {
        let stdout = run_query_in_picodata_admin(self.picodata_path, &self.socket_path, lua_query)?;

        let Some(output) = stdout.lines().find_map(|line| line.strip_prefix("- ")) else {
            bail!("unable to extract single line from Lua query output '{stdout}'");
        };

        Ok(output.to_string())
    }

    fn get_parsed_lua_output<T>(&self, lua_query: &str) -> anyhow::Result<T>
    where
        T: FromStr,
        T::Err: StdError + Send + Sync + 'static,
    {
        let raw = self
            .get_lua_single_line_output(lua_query)
            .context("failed to execute Lua query")?;

        raw.parse::<T>()
            .with_context(|| format!("failed to parse Lua output: {raw:?}"))
    }

    /// Fetches instance name from admin socket.
    pub fn instance_name(&self) -> Result<String> {
        self.get_parsed_lua_output(GET_INSTANCE_NAME)
    }

    /// Fetches state of the instance from admin socket.
    pub fn current_state(&self) -> Result<InstanceState> {
        self.get_lua_single_line_output(GET_INSTANCE_CURRENT_STATE)
            .and_then(|state| state.parse())
    }

    // Fetches map of [replicaset_uuid, bucket_count] obtained from
    // vshard router for the instance tier.
    pub fn vshard_replicaset_map(&self) -> Result<HashMap<String, u32>> {
        self.get_lua_single_line_output(GET_VSHARD_REPLICASET_MAP)
            .and_then(|o| parse_lua_json(&o))
    }

    /// Fetches configured number of buckets in the instance tier.
    pub fn tier_bucket_count(&self) -> Result<u32> {
        self.get_parsed_lua_output(GET_TIER_BUCKET_COUNT)
    }

    /// Fetches configured number of replicasets in the instance tier.
    pub fn tier_replicaset_count(&self) -> Result<u32> {
        self.get_parsed_lua_output(GET_TIER_REPLICASET_COUNT)
    }

    /// Fetches current number of buckets located on the instance
    pub fn bucket_count(&self) -> Result<u32> {
        self.get_parsed_lua_output(GET_INSTANCE_BUCKET_COUNT)
    }

    /// Fetches UUID of the instance's replicaset.
    pub fn replicaset_uuid(&self) -> Result<String> {
        self.get_parsed_lua_output(GET_REPLICASET_UUID)
    }

    /// Checks whether current instance is a master in replicaset
    /// according to `_pico_replicaset` table.
    pub fn is_master_of_replicaset(&self, replicaset_uuid: &str) -> Result<bool> {
        let instance_name = self.instance_name()?;

        let current_master_name: String = self.get_parsed_lua_output(&format!(
            "\\lua\npico.sql(\"select current_master_name from _pico_replicaset where uuid = ?\", {{ \"{replicaset_uuid}\" }}).rows[1][1]",
        ))?;
        let target_master_name: String = self.get_parsed_lua_output(&format!(
            "\\lua\npico.sql(\"select target_master_name from _pico_replicaset where uuid = ?\", {{ \"{replicaset_uuid}\" }}).rows[1][1]",
        ))?;

        if current_master_name != target_master_name {
            warn!("Master change is in progress; using target master name");
        }

        Ok(instance_name == target_master_name)
    }
}
