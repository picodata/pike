#![allow(unused)]

use constcat::concat;
use flate2::bufread::GzDecoder;
use log::info;
use regex::Regex;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixStream;
use std::path::{Component, PathBuf};
use std::thread;
use std::{
    fs::{self},
    io::ErrorKind,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tar::Archive;
use toml_edit::{DocumentMut, Item};

pub const TESTS_DIR: &str = "./tests/tmp/";
pub const PLUGIN_NAME: &str = "test-plugin";
pub const PLUGIN_DIR: &str = concat!(TESTS_DIR, PLUGIN_NAME);
pub const SHARED_TARGET_NAME: &str = "shared_target";
pub const SHARED_TARGET_PATH: &str = concat!(TESTS_DIR, SHARED_TARGET_NAME);

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
pub const LIB_EXT: &str = "dylib";

pub enum BuildType {
    Release,
    Debug,
}

#[derive(Default)]
#[allow(clippy::struct_field_names)]
pub struct CmdArguments {
    pub run_args: Vec<String>,
    pub build_args: Vec<String>,
    pub plugin_args: Vec<String>,
    pub stop_args: Vec<String>,
}

pub struct Cluster {
    run_handler: Option<Child>,
    pub cmd_args: CmdArguments,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let mut args = vec!["stop", "--plugin-path", PLUGIN_NAME];
        args.extend(self.cmd_args.stop_args.iter().map(String::as_str));
        exec_pike(args);

        if let Some(ref mut run_handler) = self.run_handler {
            run_handler.wait().unwrap();
        }
    }
}

impl Cluster {
    fn new(run_params: CmdArguments) -> Cluster {
        info!("cleaning artefacts from previous run");

        match fs::remove_file(Path::new(TESTS_DIR).join("instance.log")) {
            Ok(()) => info!("Clearing logs."),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                info!("instance.log not found, skipping cleanup");
            }
            Err(e) => panic!("failed to delete instance.log: {e}"),
        }

        Cluster {
            run_handler: None,
            cmd_args: run_params,
        }
    }

    fn set_run_handler(&mut self, handler: Child) {
        self.run_handler = Some(handler);
    }
}

pub struct TestPluginInitParams<A = String>
where
    A: AsRef<OsStr> + std::fmt::Debug,
{
    /// Plugin name for new plugin
    pub name: String,
    /// Additional args for pike new command
    pub init_args: Vec<A>,
    /// Plugin project path
    pub plugin_path: PathBuf,
    /// Shared target directory to use as a cache
    pub shared_target_path: PathBuf,
    /// Current directory to run pike in
    pub working_dir: PathBuf,
}

impl TestPluginInitParams {
    pub fn new(plugin_name: &str) -> Self {
        Self {
            name: plugin_name.to_string(),
            plugin_path: Path::new(TESTS_DIR).join(plugin_name),
            ..Default::default()
        }
    }

    pub fn new_workspace(plugin_name: &str) -> Self {
        Self {
            name: plugin_name.to_string(),
            plugin_path: Path::new(TESTS_DIR).join(plugin_name),
            init_args: vec!["--workspace".to_string()],
            ..Default::default()
        }
    }
}

impl<A> Default for TestPluginInitParams<A>
where
    A: AsRef<OsStr> + std::fmt::Debug,
{
    fn default() -> Self {
        Self {
            name: String::from(PLUGIN_NAME),
            init_args: vec![],
            plugin_path: PathBuf::from(PLUGIN_DIR),
            shared_target_path: PathBuf::from(SHARED_TARGET_PATH),
            working_dir: PathBuf::from(TESTS_DIR),
        }
    }
}

pub fn init_plugin(plugin_name: &str) {
    init_plugin_with_args(TestPluginInitParams::new(plugin_name));
}

pub fn init_plugin_workspace(plugin_name: &str) {
    init_plugin_with_args(TestPluginInitParams::new_workspace(plugin_name));
}

pub fn init_plugin_with_args<A>(init_params: TestPluginInitParams<A>)
where
    A: AsRef<OsStr> + std::fmt::Debug,
{
    // Delete plugin project directory, if it exists from previous runs
    cleanup_dir(&init_params.plugin_path);

    // Create new plugin and link target folder to shared target folder
    let default_args = vec!["plugin", "new", &init_params.name];
    let plugin_args = init_params.init_args.iter().map(A::as_ref);
    let args = default_args
        .into_iter()
        .map(str::as_ref)
        .chain(plugin_args)
        .collect::<Vec<_>>();
    exec_pike_in(args, init_params.working_dir);

    // Ensure that directory for plugin build artifacts does exist
    let shared_target_path = init_params.shared_target_path;
    if !shared_target_path.exists() {
        fs::create_dir(&shared_target_path).unwrap();
    }
    let shared_target_path = shared_target_path.canonicalize().unwrap();

    let normalized_package_name = init_params.name.replace('-', "_");
    let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");
    let lib_d_name = format!("lib{normalized_package_name}.d");
    let profile_dir_whitelist = vec![
        "build",
        "deps",
        "examples",
        "incremental",
        ".fingerprint",
        &lib_name,
        &lib_d_name,
    ];

    // Preserve build artefacts from previous run, delete other content
    clean_dir_with_exceptions(&shared_target_path, vec!["debug", "release"]);
    clean_dir_with_exceptions(&shared_target_path.join("debug"), &profile_dir_whitelist);
    clean_dir_with_exceptions(&shared_target_path.join("release"), &profile_dir_whitelist);

    // Link target dir to shared target dir
    let plugin_target_dir = init_params
        .plugin_path
        .canonicalize()
        .unwrap()
        .join("target");

    // Compute relative path for shared target directory
    let target_rel_symlink = compute_relative_symlink(&shared_target_path, &plugin_target_dir);
    dbg!(target_rel_symlink);

    symlink(shared_target_path, init_params.plugin_path.join("target")).unwrap();
}

pub fn clean_dir_with_exceptions<I, S>(path: &PathBuf, exceptions: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr> + std::fmt::Debug,
{
    if !path.exists() {
        return;
    }

    let exception_set: std::collections::HashSet<_> = exceptions
        .into_iter()
        .map(|s| s.as_ref().to_os_string())
        .collect();

    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let file_name = entry.file_name();

        if exception_set.contains(&file_name) {
            continue;
        }

        let entry_path = entry.path();
        if entry_path.is_dir() {
            fs::remove_dir_all(entry_path).unwrap();
        } else {
            fs::remove_file(entry_path).unwrap();
        }
    }
}

pub fn validate_symlink(symlink_path: &PathBuf) -> bool {
    if let Ok(metadata) = fs::symlink_metadata(symlink_path) {
        if metadata.file_type().is_symlink() {
            if let Ok(resolved_path) = fs::read_link(symlink_path) {
                return fs::metadata(resolved_path).is_ok();
            }
        }
    }

    false
}

/// Computes relative symlink to path `src_path`. `dst_path` is future symlink placement.  
/// Both paths must be absolute.
pub fn compute_relative_symlink<SrcP, DstP>(src_path: SrcP, dst_path: DstP) -> PathBuf
where
    SrcP: AsRef<Path>,
    DstP: AsRef<Path>,
{
    let (src_path, dst_path) = (src_path.as_ref(), dst_path.as_ref());
    let common_prefix = src_path
        .components()
        .zip(dst_path.components())
        .map_while(|(a, b)| Option::from(a).filter(|a| a == &b))
        .collect::<PathBuf>();
    let shared_target_part = src_path.strip_prefix(&common_prefix).unwrap().components();
    dst_path
        .strip_prefix(&common_prefix)
        .unwrap()
        .components()
        .map(|_| PathBuf::from(".."))
        .collect::<PathBuf>()
        .join(shared_target_part)
}

pub fn assert_path_existance(path: &Path, must_be_symlink: bool) {
    assert!(path.exists());

    let is_symlink = path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    if must_be_symlink {
        assert!(is_symlink);
    } else {
        assert!(!is_symlink);
    }
}

pub fn build_plugin(build_type: &BuildType, new_version: &str, plugin_path: &Path) {
    // Change plugin version
    let cargo_toml_path = plugin_path.join("Cargo.toml");
    let toml_content = fs::read_to_string(&cargo_toml_path).unwrap();

    let mut doc = toml_content
        .parse::<DocumentMut>()
        .expect("Failed to parse Cargo.toml");

    if let Some(Item::Table(package)) = doc.get_mut("package") {
        if let Some(version) = package.get_mut("version") {
            *version = toml_edit::value(new_version);
        }
    }
    fs::write(cargo_toml_path, doc.to_string()).unwrap();

    // Build according version
    let output = match build_type {
        BuildType::Debug => Command::new("cargo")
            .arg("build")
            .current_dir(plugin_path)
            .output()
            .unwrap(),
        BuildType::Release => Command::new("cargo")
            .arg("build")
            .arg("--release")
            .current_dir(plugin_path)
            .output()
            .unwrap(),
    };

    if !output.status.success() {
        io::stdout().write_all(&output.stdout).unwrap();
        io::stderr().write_all(&output.stderr).unwrap();

        assert!(output.status.code().unwrap() != 0);
    }
}

pub fn run_cluster(
    timeout: Duration,
    total_instances: i32,
    cmd_args: CmdArguments,
) -> Result<Cluster, std::io::Error> {
    // Set up cleanup function
    let mut cluster_handle = Cluster::new(cmd_args);

    // Create plugin from template
    let mut args = cluster_handle
        .cmd_args
        .plugin_args
        .iter()
        .map(String::as_str);

    init_plugin_with_args(TestPluginInitParams {
        name: "test-plugin".to_string(),
        init_args: args.collect(),
        ..Default::default()
    });

    // Build the plugin
    Command::new("cargo")
        .arg("build")
        .args(&cluster_handle.cmd_args.build_args)
        .current_dir(PLUGIN_DIR)
        .output()?;

    // Setup the cluster
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let run_handler = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .arg("run")
        .args(&cluster_handle.cmd_args.run_args)
        .current_dir(PLUGIN_DIR)
        .spawn()
        .unwrap();
    cluster_handle.set_run_handler(run_handler);

    let start_time = Instant::now();

    // Run in the loop until we get info about successful plugin installation
    loop {
        // Get path to data dir from cmd_args
        let cur_run_args = &cluster_handle.cmd_args.run_args;
        let mut data_dir_path = Path::new("tmp");
        if let Some(index) = cur_run_args.iter().position(|x| x == "--data-dir") {
            if index + 1 < cur_run_args.len() {
                data_dir_path = Path::new(&cur_run_args[index + 1]);
            }
        }
        // Check if cluster set up correctly
        let mut picodata_admin = await_picodata_admin(
            Duration::from_secs(60),
            Path::new(PLUGIN_DIR),
            data_dir_path,
        )?;
        let stdout = picodata_admin
            .stdout
            .take()
            .expect("Failed to capture stdout");

        assert!(start_time.elapsed() < timeout, "cluster setup timeouted");

        let queries = vec![
            r"SELECT enabled FROM _pico_plugin;",
            r"SELECT current_state FROM _pico_instance;",
            r"\help;",
        ];

        // New scope to avoid infinite cycle while reading picodata stdout
        {
            let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();
            for query in queries {
                picodata_stdin.write_all(query.as_bytes()).unwrap();
            }
            picodata_admin.wait().unwrap();
        }

        let mut plugin_ready = false;
        let mut can_connect = false;
        let mut online_instances_counter = 0;

        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line.expect("failed to read picodata stdout");
            if line.contains("true") {
                plugin_ready = true;
            }
            if line.contains("Connected to admin console by socket") {
                can_connect = true;
            }
            if line.contains("Online") {
                online_instances_counter += 1;
            }
        }

        picodata_admin.kill().unwrap();

        if can_connect && plugin_ready && online_instances_counter == total_instances {
            return Ok(cluster_handle);
        }

        thread::sleep(Duration::from_secs(5));
    }
}

pub struct ClusterStateToCheck<'a> {
    pub pico_instance: &'a str,
    pub pico_plugin: &'a str,
}

pub fn wait_cluster_start_completed<P, CheckFn>(plugin_path: P, state_check_fn: CheckFn) -> bool
where
    P: AsRef<Path>,
    for<'a> CheckFn: Fn(ClusterStateToCheck<'a>) -> bool,
{
    let start = Instant::now();
    let mut cluster_started = false;
    while Instant::now().duration_since(start) < Duration::from_secs(60) {
        let pico_instance =
            get_picodata_table(plugin_path.as_ref(), Path::new("tmp"), "_pico_instance");
        let pico_plugin =
            get_picodata_table(plugin_path.as_ref(), Path::new("tmp"), "_pico_plugin");
        let current_state = ClusterStateToCheck {
            pico_instance: &pico_instance,
            pico_plugin: &pico_plugin,
        };
        let check_fn = std::panic::AssertUnwindSafe(|| state_check_fn(current_state));
        let check_result = std::panic::catch_unwind(check_fn);
        if let Ok(value) = check_result {
            cluster_started = value;
            break;
        }
    }
    cluster_started
}

pub fn get_picodata_table(plugin_path: &Path, data_dir_path: &Path, table_name: &str) -> String {
    let mut picodata_admin =
        await_picodata_admin(Duration::from_secs(60), plugin_path, data_dir_path).unwrap();

    // New scope to avoid infinite cycle while reading picodata stdout
    {
        let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();
        let query = format!(r"SELECT * FROM {table_name};");
        picodata_stdin.write_all(query.as_bytes()).unwrap();
        picodata_admin.wait().unwrap();
    }

    let mut stderr = picodata_admin
        .stderr
        .take()
        .expect("Failed to capture stderr");

    let mut error_message = String::new();
    stderr.read_to_string(&mut error_message).unwrap();
    assert!(
        error_message.is_empty(),
        "Error in picodata: {error_message}"
    );

    let stdout = picodata_admin
        .stdout
        .take()
        .expect("Failed to capture stdout");

    let reader = BufReader::new(stdout);
    reader
        .lines()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
        .join("\n")
}

fn set_current_version_of_pike(plugin_path: &OsStr) {
    let cargo_path = Path::new(TESTS_DIR).join(plugin_path).join("Cargo.toml");
    let Ok(cargo_content) = fs::read_to_string(&cargo_path) else {
        return;
    };
    let re = Regex::new(r"picodata-pike =.*").unwrap();
    let cargo_with_fixed_pike =
        re.replace_all(&cargo_content, r#"picodata-pike = { path = "../../.." }"#);
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(cargo_path)
        .unwrap();
    writeln!(&file, "{cargo_with_fixed_pike}").unwrap();
}

pub fn exec_pike<I, S>(args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr> + std::fmt::Debug,
{
    exec_pike_in(args, TESTS_DIR);
}

// Spawn child process where pike is executed
// Funciton waits for child process to end
pub fn exec_pike_in<I, S, WD>(args: I, work_dir: WD)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr> + std::fmt::Debug,
    WD: AsRef<Path> + std::fmt::Debug,
{
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let args: Vec<S> = args.into_iter().collect();

    dbg!(&work_dir, &args);

    if let Some(plugin_path_pos) = args.iter().position(|a| a.as_ref() == "--plugin-path") {
        set_current_version_of_pike(args[plugin_path_pos + 1].as_ref());
    };

    let mut pike_child = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .args(args)
        .current_dir(work_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to execute pike");

    let status = pike_child.wait().unwrap();

    let outputs: [Box<dyn Read + Send>; 2] = [
        Box::new(pike_child.stdout.unwrap()),
        Box::new(pike_child.stderr.unwrap()),
    ];
    for output in outputs {
        let reader = BufReader::new(output);
        for line in reader.lines() {
            let line = line.expect("failed to read picodata admin output");
            println!("{line}");
        }
    }

    assert!(status.success(), "pike run failed");
}

pub fn wait_for_proc(proc: &mut Child, timeout: Duration) {
    let start_time = Instant::now();

    loop {
        assert!(
            start_time.elapsed() < timeout,
            "Process hanging for too long"
        );

        match proc.try_wait().unwrap() {
            Some(_) => {
                break;
            }
            None => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}

pub fn await_picodata_admin(
    timeout: Duration,
    plugin_path: &Path,
    data_dir_path: &Path,
) -> Result<Child, std::io::Error> {
    let start_time = Instant::now();
    loop {
        assert!(
            start_time.elapsed() < timeout,
            "process hanging for too long"
        );

        let picodata_admin = Command::new("picodata")
            .arg("admin")
            .arg(
                plugin_path
                    .join(data_dir_path)
                    .join("cluster/i1/admin.sock"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        match picodata_admin {
            Ok(process) => {
                info!("successfully connected to picodata cluster.");
                return Ok(process);
            }
            Err(_) => {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Recursively deletes directory, if exists.  
/// Does not follow symlinks.
pub fn cleanup_dir(path: &Path) {
    match fs::remove_dir_all(path) {
        Ok(()) => info!("clearing test plugin dir."),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            info!("plugin dir not found, skipping cleanup");
        }
        Err(e) => panic!("failed to delete plugin_dir: {e}"),
    }
}

pub fn unpack_archive(path: &Path, unpack_to: &Path) {
    let tar_archive = File::open(path).unwrap();
    let buf_reader = BufReader::new(tar_archive);
    let decompressor = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(decompressor);

    archive.unpack(unpack_to).unwrap();
}

pub fn is_instance_running(instance_dir: &Path) -> bool {
    let socket_path = instance_dir.join("admin.sock");
    socket_path.exists() && UnixStream::connect(&socket_path).is_ok()
}
