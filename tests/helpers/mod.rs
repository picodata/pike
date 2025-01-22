use constcat::concat;
use log::info;
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::thread;
use std::{
    fs::{self},
    io::ErrorKind,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub const TESTS_DIR: &str = "./tests/tmp/";
pub const PLUGIN_DIR: &str = concat!(TESTS_DIR, "test_plugin/");

pub struct Cluster {
    run_handler: Option<Child>,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        let mut child = run_pike(vec!["stop"], PLUGIN_DIR).unwrap();
        child.wait().unwrap();
        if let Some(ref mut run_handler) = self.run_handler {
            run_handler.wait().unwrap();
        }
    }
}

impl Cluster {
    fn new() -> Cluster {
        info!("cleaning artefacts from previous run");

        match fs::remove_file(Path::new(TESTS_DIR).join("instance.log")) {
            Ok(()) => info!("Clearing logs."),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                info!("instance.log not found, skipping cleanup");
            }
            Err(e) => panic!("failed to delete instance.log: {e}"),
        }

        match fs::remove_dir_all(PLUGIN_DIR) {
            Ok(()) => info!("clearing test plugin dir."),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                info!("plugin dir not found, skipping cleanup");
            }
            Err(e) => panic!("failed to delete plugin_dir: {e}"),
        }

        Cluster { run_handler: None }
    }

    fn set_run_handler(&mut self, handler: Child) {
        self.run_handler = Some(handler);
    }
}

pub fn run_cluster(timeout: Duration, total_instances: i32) -> Result<Cluster, std::io::Error> {
    // Set up cleanup function
    let mut cluster_handle = Cluster::new();

    // Create plugin from template
    let mut plugin_creation_proc =
        run_pike(vec!["plugin", "new", "test_plugin"], TESTS_DIR).unwrap();

    wait_for_proc(&mut plugin_creation_proc, Duration::from_secs(10));

    // Build the plugin
    Command::new("cargo")
        .args(vec!["build"])
        .current_dir(PLUGIN_DIR)
        .output()?;

    // Setup the cluster
    let run_handler = run_pike(vec!["run"], PLUGIN_DIR).unwrap();
    cluster_handle.set_run_handler(run_handler);

    let start_time = Instant::now();

    // Run in the loop until we get info about successful plugin installation
    loop {
        // Check if cluster set up correctly
        let mut picodata_admin = await_picodata_admin(Duration::from_secs(60))?;
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

pub fn run_pike<A, P>(args: Vec<A>, current_dir: P) -> Result<std::process::Child, std::io::Error>
where
    A: AsRef<OsStr>,
    P: AsRef<Path>,
{
    let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    Command::new(format!("{root_dir}/target/debug/cargo-pike"))
        .arg("pike")
        .args(args)
        .current_dir(current_dir)
        .spawn()
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

pub fn await_picodata_admin(timeout: Duration) -> Result<Child, std::io::Error> {
    let start_time = Instant::now();

    loop {
        assert!(
            start_time.elapsed() < timeout,
            "process hanging for too long"
        );

        let picodata_admin = Command::new("picodata")
            .arg("admin")
            .arg(PLUGIN_DIR.to_string() + "tmp/cluster/i_1/admin.sock")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
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
