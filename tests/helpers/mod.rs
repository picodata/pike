#![allow(unused)]

use constcat::concat;
use flate2::bufread::GzDecoder;
use log::info;
use regex::Regex;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::symlink;
use std::path::PathBuf;
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

#[cfg(target_os = "linux")]
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

pub fn init_plugin(plugin_name: &str) {
    init_plugin_with_args(plugin_name, vec![]);
}

pub fn init_plugin_workspace(plugin_name: &str) {
    init_plugin_with_args(plugin_name, vec!["--workspace"]);
}

fn init_plugin_with_args(plugin_name: &str, plugin_args: Vec<&str>) {
    let plugin_path = Path::new(TESTS_DIR).join(plugin_name);
    cleanup_dir(&plugin_path);

    // Create new plugin and link target folder to shared target folder
    let mut args = vec!["plugin", "new", plugin_name];
    args.extend(plugin_args);
    exec_pike(args);

    let shared_target_path = &Path::new(TESTS_DIR).join(SHARED_TARGET_NAME);
    if !shared_target_path.exists() {
        fs::create_dir(shared_target_path).unwrap();
    }

    let normalized_package_name = plugin_name.replace('-', "_");
    let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");
    let lib_d_name = format!("lib{normalized_package_name}.d");

    // Save build artefacts from previous run
    clean_dir_with_exceptions(shared_target_path, vec!["debug", "release"]);

    let build_exceptions = vec![
        "build",
        "deps",
        "examples",
        "incremental",
        ".fingerprint",
        &lib_name,
        &lib_d_name,
    ];

    clean_dir_with_exceptions(&shared_target_path.join("debug"), &build_exceptions);
    clean_dir_with_exceptions(&shared_target_path.join("release"), &build_exceptions);

    // Link target dir to shared target dir
    // Destination path for symlink is relative to plugin folder
    symlink(
        Path::new("../").join(SHARED_TARGET_NAME),
        plugin_path.join("target"),
    )
    .unwrap();
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

    init_plugin_with_args("test-plugin", args.collect());

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

// Spawn child process where pike is executed
// Funciton waits for child process to end
pub fn exec_pike<I, S>(args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr> + std::fmt::Debug,
{
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let args: Vec<S> = args.into_iter().collect();

    dbg!(&args);
    dbg!(TESTS_DIR);

    if let Some(plugin_path_pos) = args.iter().position(|a| a.as_ref() == "--plugin-path") {
        set_current_version_of_pike(args[plugin_path_pos + 1].as_ref());
    };

    let mut pike_child = Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .args(args)
        .current_dir(TESTS_DIR)
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
