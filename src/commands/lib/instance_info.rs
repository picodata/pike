use crate::commands::lib::run_query_in_picodata_admin;
use anyhow::{bail, Context, Result};
use log::warn;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
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

const REBALANCER_WAKEUP: &str = "\\lua\nvshard.storage.rebalancer_wakeup()";
const ROUTER_DISCOVERY_WAKEUP: &str = "\\lua\n\
for _, router in pairs(vshard.router.internal.routers) do \
    router:discovery_wakeup(); \
end";

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

    pub fn is_expelled(self) -> bool {
        matches!(self, InstanceState::Expelled)
    }
}

impl FromStr for InstanceState {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let state = match s.to_ascii_lowercase().as_str() {
            "online" => Self::Online,
            "offline" => Self::Offline,
            "expelled" => Self::Expelled,
            unknown => bail!("Unknown instance state variant: '{unknown}'"),
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

    /// Runs input query in picodata admin and expects empty output.
    fn get_lua_empty_output(&self, lua_query: &str) -> Result<()> {
        let stdout = run_query_in_picodata_admin(self.picodata_path, &self.socket_path, lua_query)?;

        let mut lines = stdout.lines();
        let Some(first) = lines.find_map(|line| line.strip_prefix("- error:")) else {
            return Ok(());
        };

        // The message continues until the end of the YAML document.
        let message = std::iter::once(first)
            .chain(lines.take_while(|line| *line != "..."))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" ");

        bail!("Lua query failed: {message}");
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

    /// Expels the instance with the given picodata name from the cluster.
    pub fn expel(&self, instance_name: &str, force: bool, timeout: Duration) -> Result<()> {
        let expelled: bool = self.get_parsed_lua_output(&format!(
            "\\lua\nreturn pico.expel('{instance_name}', {{ force = {force}, timeout = {} }})",
            timeout.as_secs()
        ))?;
        if !expelled {
            bail!("picodata refused to expel instance '{instance_name}'");
        }

        Ok(())
    }

    /// Fetches state of the instance with the given picodata name
    pub fn state_of(&self, instance_name: &str) -> Result<InstanceState> {
        self.get_lua_single_line_output(&format!(
            "\\lua\nreturn box.space._pico_instance:get('{instance_name}').current_state[1]"
        ))
        .and_then(|state| state.parse())
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

    pub fn rebalancer_wakeup(&self) -> Result<()> {
        self.get_lua_empty_output(REBALANCER_WAKEUP)
    }

    pub fn router_discovery_wakeup(&self) -> Result<()> {
        self.get_lua_empty_output(ROUTER_DISCOVERY_WAKEUP)
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
            .context("unable to determine replicaset uuid")
    }

    /// Checks whether current instance is a master in replicaset
    /// according to `_pico_replicaset` table.
    pub fn is_master_of_replicaset(&self, replicaset_uuid: &str) -> Result<bool> {
        let error_context =
            || format!("unable to determine master of replicaset '{replicaset_uuid}'");

        let instance_name = self.instance_name().with_context(error_context)?;

        let current_master_name: String = self.get_parsed_lua_output(&format!(
            "\\lua\npico.sql(\"select current_master_name from _pico_replicaset where uuid = ?\", {{ \"{replicaset_uuid}\" }}).rows[1][1]",
        )).with_context(error_context)?;
        let target_master_name: String = self.get_parsed_lua_output(&format!(
            "\\lua\npico.sql(\"select target_master_name from _pico_replicaset where uuid = ?\", {{ \"{replicaset_uuid}\" }}).rows[1][1]",
        )).with_context(error_context)?;

        if current_master_name != target_master_name {
            warn!("Master change is in progress; using target master name");
        }

        Ok(instance_name == target_master_name)
    }
}
