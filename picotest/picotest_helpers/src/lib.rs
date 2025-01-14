use constcat::concat;
use log::info;
use rand::distributions::Alphanumeric;
use rand::Rng;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::thread;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const TESTS_DIR: &str = "../tmp/";
pub const PLUGIN_DIR: &str = concat!(TESTS_DIR, "test_plugin/");

#[derive(Debug)]
pub struct Cluster {
    uuid: Uuid,
    data_dir: String,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Cluster {
    pub fn new(data_dir: String) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            data_dir,
        }
    }

    pub fn stop(&self) {
        run_pike(vec!["stop", "--data-dir", &self.data_dir], PLUGIN_DIR).unwrap();
        thread::sleep(Duration::from_secs(5));
        let _ = fs::remove_dir_all(self.data_dir.to_owned());
    }
}

pub fn run_cluster() -> Result<Cluster, std::io::Error> {
    let data_dir = tmp_dir();
    let cluster_handle = Cluster::new(data_dir.clone());

    let timeout = Duration::from_secs(60);
    run_pike(vec!["run", "--data-dir", &data_dir], PLUGIN_DIR).unwrap();

    let start_time = Instant::now();
    // Run in the loop until we get info about successful plugin installation
    loop {
        // Check if cluster set up correctly
        let mut picodata_admin = await_picodata_admin(Duration::from_secs(60), &data_dir)?;
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

        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line.expect("failed to read picodata stdout");
            if line.contains("true") {
                plugin_ready = true;
            }
            if line.contains("Connected to admin console by socket") {
                can_connect = true;
            }
        }

        picodata_admin.kill().unwrap();
        if can_connect && plugin_ready {
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
    // let root_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    Command::new(format!("cargo"))
        .arg("pike")
        .args(args)
        .current_dir(current_dir)
        .spawn()
}

pub fn await_picodata_admin(timeout: Duration, data_dir: &str) -> Result<Child, std::io::Error> {
    let start_time = Instant::now();

    loop {
        assert!(
            start_time.elapsed() < timeout,
            "process hanging for too long"
        );

        let picodata_admin = Command::new("picodata")
            .arg("admin")
            .arg(PLUGIN_DIR.to_string() + data_dir + "/cluster/i_1/admin.sock")
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

pub fn tmp_dir() -> String {
    let mut rng = rand::thread_rng();
    format!(
        "./tmp/tests/{}",
        (0..8)
            .map(|_| rng.sample(Alphanumeric))
            .map(char::from)
            .collect::<String>()
    )
}
