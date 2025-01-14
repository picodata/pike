use log::info;
use rand::distributions::Alphanumeric;
use rand::Rng;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::thread;
use std::{
    io::Error,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use uuid::Uuid;

const SOCKET_PATH: &str = "cluster/i_1/admin.sock";

#[derive(Debug)]
pub struct Cluster {
    pub uuid: Uuid,
    pub path: String,
    pub data_dir: String,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Cluster {
    pub fn new(path: String, data_dir: String) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            path,
            data_dir,
        }
    }

    pub fn stop(&self) {
        run_pike(vec!["stop", "--data-dir", &self.data_dir], &self.path).unwrap();
        thread::sleep(Duration::from_secs(5));
        let _ = fs::remove_dir_all(self.plugin_path());
    }

    pub fn run(self) -> Result<Self, Error> {
        run_pike(vec!["run", "--data-dir", &self.data_dir], &self.path).unwrap();
        self.wait()
    }

    pub fn recreate(self) -> Result<Self, Error> {
        self.stop();
        self.run()
    }

    fn wait(self) -> Result<Self, Error> {
        let timeout = Duration::from_secs(60);
        let start_time = Instant::now();

        loop {
            let mut picodata_admin: Child = self.await_picodata_admin()?;
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
                return Ok(self);
            }

            thread::sleep(Duration::from_secs(5));
        }
    }

    pub fn run_query<T: AsRef<[u8]>>(&self, query: T) -> Result<String, Error> {
        let mut picodata_admin = self.await_picodata_admin()?;

        let stdout = picodata_admin
            .stdout
            .take()
            .expect("Failed to capture stdout");
        {
            let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();

            picodata_stdin.write_all(query.as_ref()).unwrap();
            picodata_admin.wait().unwrap();
        }

        let mut result = String::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => result.push_str(&l),
                Err(e) => return Err(e),
            }
        }
        picodata_admin.kill()?;

        Ok(result)
    }

    fn await_picodata_admin(&self) -> Result<Child, Error> {
        let timeout = Duration::from_secs(60);
        let start_time = Instant::now();
        loop {
            assert!(
                start_time.elapsed() < timeout,
                "process hanging for too long"
            );

            let picodata_admin = Command::new("picodata")
                .arg("admin")
                .arg(self.socket_path())
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

    pub fn plugin_path(&self) -> String {
        format!("{}/{}", &self.path, &self.data_dir)
    }

    pub fn socket_path(&self) -> String {
        format!("{}/{}", &self.plugin_path(), SOCKET_PATH)
    }
}

pub fn run_cluster(path: &str) -> Result<Cluster, Error> {
    let data_dir = tmp_dir();
    let cluster = Cluster::new(path.to_owned(), data_dir.to_owned());
    cluster.run()
}

pub fn run_pike<A, P>(args: Vec<A>, current_dir: P) -> Result<std::process::Child, Error>
where
    A: AsRef<OsStr>,
    P: AsRef<Path>,
{
    Command::new("cargo")
        .arg("pike")
        .args(args)
        .current_dir(current_dir)
        .spawn()
}

pub fn tmp_dir() -> String {
    let mut rng = rand::thread_rng();
    format!(
        "tmp/tests/{}",
        (0..8)
            .map(|_| rng.sample(Alphanumeric))
            .map(char::from)
            .collect::<String>()
    )
}
