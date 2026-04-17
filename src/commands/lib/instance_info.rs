use crate::commands::lib::run_query_in_picodata_admin;
use anyhow::{bail, Context, Result};
use std::error::Error as StdError;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

const ADMIN_SOCKET_FILENAME: &str = "admin.sock";

const GET_INSTANCE_NAME: &str = "\\lua\npico.instance_info().name";
const GET_INSTANCE_CURRENT_STATE: &str = "\\lua\npico.instance_info().current_state.variant";

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
}
