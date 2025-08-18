use anyhow::{bail, Context, Result};
use flate2::bufread::GzDecoder;
use fs_extra::dir;
use std::fmt::Display;
use std::fs::{self, File, FileType};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tar::Archive;

#[cfg(target_os = "linux")]
pub const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
pub const LIB_EXT: &str = "dylib";

#[derive(Clone, Copy, Debug)]
pub enum BuildType {
    Release,
    Debug,
}

impl Display for BuildType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BuildType::Release => "release",
            BuildType::Debug => "debug",
        })
    }
}

pub fn is_plugin_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    if !path.join("Cargo.toml").exists() {
        return false;
    }

    if path.join("manifest.yaml.template").exists() {
        return true;
    }

    fs::read_dir(path)
        .unwrap()
        .filter(Result::is_ok)
        .map(|e| e.unwrap().path())
        .filter(|e| e.is_dir())
        .any(|dir| dir.join("manifest.yaml.template").exists())
}

pub fn is_plugin_shipping_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!("path is not a plugin shipping directory");
    }
    let versioned_readers = fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|version| version.file_type().as_ref().is_ok_and(FileType::is_dir))
        .map(|path| fs::read_dir(path.path()));
    for version in versioned_readers.flatten() {
        let root_files = version.filter_map(Result::ok).collect::<Vec<_>>();
        let has_manifest = root_files.iter().any(|path| {
            path.file_name() == "manifest.yaml"
                && path.file_type().as_ref().is_ok_and(FileType::is_file)
        });
        if has_manifest {
            return Ok(());
        }
    }
    bail!("path does not match plugin dir structure")
}

/// Checks if provided path contains valid packed plugin archive
pub fn is_plugin_archive(test_path: &Path) -> Result<()> {
    if !test_path.is_file() {
        bail!("plugin archive path must be a file");
    }
    let file = File::options()
        .read(true)
        .write(false)
        .create(false)
        .open(test_path)
        .context("unable to open plugin archive candidate")?;
    let buf_reader = BufReader::new(file);
    let file_untar = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(file_untar);
    let Ok(archive_entries) = archive.entries() else {
        bail!("unable to read plugin archive candidate");
    };
    let mut has_manifest = false;
    let mut has_lib = false;
    let lib_suffix = format!(".{LIB_EXT}");
    for entry in archive_entries.filter_map(Result::ok) {
        if let Ok(entry_path) = entry.path() {
            // plugin_name / plugin_version / root_file_name
            if entry_path.components().count() == 3 {
                if let Some(last_part) = entry_path.components().last() {
                    has_manifest = has_manifest || last_part.as_os_str() == "manifest.yaml";
                    has_lib = has_lib
                        || last_part
                            .as_os_str()
                            .to_string_lossy()
                            .ends_with(&lib_suffix);
                }
            }
        }
        if has_manifest && has_lib {
            return Ok(());
        }
    }
    if !has_manifest {
        bail!("plugin archive candidate missing manifest");
    }
    if !has_lib {
        bail!("plugin archive candidate missing plugin library");
    }
    bail!("plugin archive candidate has invalid structure");
}

#[allow(clippy::needless_pass_by_value)]
pub fn cargo_build(build_type: BuildType, target_dir: &PathBuf, build_dir: &PathBuf) -> Result<()> {
    let mut args = vec!["build"];
    if let BuildType::Release = build_type {
        args.push("--release");
    }

    let mut child = Command::new("cargo")
        .args(args)
        .arg("--target-dir")
        .arg(target_dir)
        .stdout(Stdio::piped())
        .current_dir(build_dir)
        .spawn()
        .context("running cargo build")?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = line.unwrap_or_else(|e| format!("{e}"));
        print!("{line}");
    }

    if !child.wait().unwrap().success() {
        let mut stderr = String::new();
        child.stderr.unwrap().read_to_string(&mut stderr).unwrap();
        bail!("build error: {stderr}");
    }

    Ok(())
}

// Return socket path to active instance
pub fn get_active_socket_path(
    data_dir: &Path,
    plugin_path: &Path,
    instance_name: &str,
) -> Option<PathBuf> {
    let socket_path = plugin_path
        .join(data_dir)
        .join("cluster")
        .join(instance_name)
        .join("admin.sock");

    if socket_path.exists() && UnixStream::connect(&socket_path).is_ok() {
        return Some(socket_path);
    }

    None
}

// Scan data directory and return the first active instance's socket path
pub fn find_active_socket_path(data_dir: &Path, plugin_path: &Path) -> Result<Option<PathBuf>> {
    let instances_path = plugin_path.join(data_dir.join("cluster"));
    if !instances_path.exists() {
        return Ok(None);
    }

    let dirs = fs::read_dir(&instances_path).context(format!(
        "cluster data dir with path {} does not exist",
        instances_path.to_string_lossy()
    ))?;

    for current_dir in dirs {
        let dir_name = current_dir?.file_name();
        if let Some(name) = dir_name.to_str() {
            let socket_path = get_active_socket_path(data_dir, plugin_path, name);
            if socket_path.is_some() {
                return Ok(socket_path);
            }
        }
    }

    Ok(None)
}

/// Validates and unpacks plugin(s) from shipping archive into destination path,
/// preserving archive structure. Does not create destination path itself.
pub fn unpack_shipping_archive(src_path: &Path, dst_path: &Path) -> Result<()> {
    is_plugin_archive(src_path).with_context(|| {
        let (from, to) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("can not unpack shipping archive at {from} to {to}")
    })?;

    let file = File::options()
        .read(true)
        .write(false)
        .create(false)
        .open(src_path)
        .context("unable to open plugin archive")?;
    let buf_reader = BufReader::new(file);
    let decompressor = GzDecoder::new(buf_reader);

    // by default - override existing, preserve mtime
    let mut archive = Archive::new(decompressor);
    archive.unpack(dst_path).with_context(|| {
        let (from, to) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("failed to unpack shipping archive at {from} to {to}")
    })?;
    Ok(())
}

/// Copies directory at `src_path` into `dst_dir`
pub fn copy_directory_tree(src_path: &Path, dst_dir: &Path) -> Result<()> {
    let src_path = src_path.canonicalize().with_context(|| {
        let src_path = src_path.to_string_lossy();
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("<unknown>"));
        let current_dir = current_dir.to_string_lossy();
        format!("path {src_path} does not exists or not a directory (pwd {current_dir})")
    })?;
    let opts = dir::CopyOptions::default().overwrite(true);
    dir::copy(&src_path, dst_dir, &opts).with_context(|| {
        let (src_path, dst_path) = (src_path.to_string_lossy(), dst_dir.to_string_lossy());
        format!("failed to copy directory tree from {src_path} to {dst_path}")
    })?;
    Ok(())
}

/// Spawns picodata admin in a new process.
pub fn spawn_picodata_admin(picodata_path: &Path, socket_path: &Path) -> Result<Child> {
    Command::new(picodata_path)
        .arg("admin")
        .arg(socket_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn child proccess of picodata admin")
}

/// Sends text to admin.sock and returns received stdout.
pub fn run_query_in_picodata_admin(
    picodata_path: &Path,
    socket_path: &Path,
    query: &str,
) -> Result<String> {
    let mut picodata_admin = spawn_picodata_admin(picodata_path, socket_path)?;
    {
        let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();
        picodata_stdin
            .write_all(query.as_bytes())
            .context("failed to send text in admin socket")?;
    }

    let exit_code = picodata_admin
        .wait()
        .context("failed to wait for picodata admin")?;

    if !exit_code.success() {
        let mut stderr = String::new();
        picodata_admin
            .stderr
            .unwrap()
            .read_to_string(&mut stderr)
            .context("failed to read stderr of picodata admin child")?;
        bail!("failed to run query in picodata admin: {stderr}");
    }

    let mut stdout = String::new();
    picodata_admin
        .stdout
        .unwrap()
        .read_to_string(&mut stdout)
        .context("failed to read stdout of picodata admin child")?;

    Ok(stdout)
}

pub mod instance_info {

    use crate::commands::lib::{find_active_socket_path, run_query_in_picodata_admin};
    use anyhow::{anyhow, bail, Context, Result};
    use std::{path::Path, str::FromStr};

    const GET_INSTANCE_NAME: &str = "\\lua\npico.instance_info().name";
    const GET_INSTANCE_CURRENT_STATE: &str = "\\lua\npico.instance_info().current_state.variant";
    const GET_CLUSTER_LEADER_ID: &str =
        "\\lua\nbox.func[\".proc_runtime_info\"]:call().raft.leader_id";

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

    pub fn get_cluster_leader_id(
        picodata_path: &Path,
        data_dir: &Path,
        plugin_path: &Path,
    ) -> Result<usize> {
        let Some(socket_path) = find_active_socket_path(data_dir, plugin_path)? else {
            bail!("failed to get cluster leader id information: no active socket found")
        };

        get_lua_single_line_output(picodata_path, &socket_path, GET_CLUSTER_LEADER_ID)
            .and_then(|str| str.parse().context("failed to parse leader id from string"))
            .map_err(|err| anyhow!("unable to get cluster leader id: {err}"))
    }
}
