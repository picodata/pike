use anyhow::{bail, Context, Result};
use colored::Colorize;
use derive_builder::Builder;
use lib::cargo_build;
use log::{error, info};
use rand::Rng;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{exit, Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{fs, path::PathBuf};

use crate::commands::lib;

#[derive(Debug, Deserialize)]
struct Tier {
    replicasets: u8,
    replication_factor: u8,
}

#[derive(Debug, Deserialize)]
struct MigrationContextVar {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct Service {
    tiers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Plugin {
    #[serde(default)]
    migration_context: Vec<MigrationContextVar>,
    #[serde(default)]
    #[serde(rename = "service")]
    services: BTreeMap<String, Service>,
    #[serde(skip)]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Topology {
    #[serde(rename = "tier")]
    tiers: BTreeMap<String, Tier>,
    #[serde(default)]
    #[serde(rename = "plugin")]
    plugins: BTreeMap<String, Plugin>,
}

impl Topology {
    fn find_plugin_versions(&mut self, plugins_dir: &Path) -> Result<()> {
        for (plugin_name, plugin) in &mut self.plugins {
            let current_plugin_dir = plugins_dir.join(plugin_name);

            if !current_plugin_dir.exists() {
                bail!(
                    "plugin directory {} does not exist",
                    current_plugin_dir.display()
                );
            }
            let mut versions: Vec<_> = fs::read_dir(current_plugin_dir)
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            versions.sort_by_key(std::fs::DirEntry::path);
            let newest_version = versions
                .last()
                .unwrap()
                .file_name()
                .to_str()
                .unwrap()
                .to_string();
            plugin.version = Some(newest_version);
        }
        Ok(())
    }
}

fn enable_plugins(topology: &Topology, data_dir: &Path, picodata_path: &PathBuf) -> Result<()> {
    let mut queries: Vec<String> = Vec::new();

    for (plugin_name, plugin) in &topology.plugins {
        let plugin_version = plugin.version.as_ref().unwrap();

        // create plugin
        queries.push(format!(
            r#"CREATE PLUGIN "{plugin_name}" {plugin_version};"#
        ));

        // add migration context
        for migration_env in &plugin.migration_context {
            queries.push(format!(
                "ALTER PLUGIN {plugin_name} {plugin_version} SET migration_context.{}='{}';",
                migration_env.name, migration_env.value
            ));
        }

        // run migrations
        queries.push(format!(
            r#"ALTER PLUGIN "{plugin_name}" MIGRATE TO {plugin_version};"#
        ));

        // add services to tiers
        for (service_name, service) in &plugin.services {
            for tier_name in &service.tiers {
                queries.push(format!(r#"ALTER PLUGIN "{plugin_name}" {plugin_version} ADD SERVICE "{service_name}" TO TIER "{tier_name}";"#));
            }
        }

        // enable plugin
        queries.push(format!(
            r#"ALTER PLUGIN "{plugin_name}" {plugin_version} ENABLE;"#
        ));
    }

    let admin_soket = data_dir.join("cluster").join("i1").join("admin.sock");

    for query in queries {
        log::info!("picodata admin: {query}");

        let mut picodata_admin = Command::new(picodata_path)
            .arg("admin")
            .arg(admin_soket.to_str().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn child proccess of picodata admin")?;

        {
            let picodata_stdin = picodata_admin.stdin.as_mut().unwrap();
            picodata_stdin
                .write_all(query.as_bytes())
                .context("failed to send plugin installation queries")?;
        }

        picodata_admin
            .wait()
            .context("failed to wait for picodata admin")?;

        let outputs: [Box<dyn Read + Send>; 2] = [
            Box::new(picodata_admin.stdout.unwrap()),
            Box::new(picodata_admin.stderr.unwrap()),
        ];
        for output in outputs {
            let reader = BufReader::new(output);
            for line in reader.lines() {
                let line = line.expect("failed to read picodata admin output");
                log::info!("picodata admin: {line}");
            }
        }
    }

    for (plugin_name, plugin) in &topology.plugins {
        info!(
            "Plugin {plugin_name}:{} is enabled",
            plugin.version.as_ref().unwrap()
        );
    }

    Ok(())
}

pub struct PicodataInstance {
    instance_name: String,
    tier: String,
    log_threads: Option<Vec<JoinHandle<()>>>,
    child: Child,
    daemon: bool,
    disable_colors: bool,
    data_dir: PathBuf,
    log_file_path: PathBuf,
}

impl PicodataInstance {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instance_id: u16,
        bin_port: u16,
        http_port: u16,
        pg_port: u16,
        first_instance_bin_port: u16,
        plugins_dir: &Path,
        replication_factor: u8,
        tier: &str,
        run_params: &Params,
    ) -> Result<Self> {
        let instance_name = format!("i{instance_id}");
        let instance_data_dir = run_params.data_dir.join("cluster").join(&instance_name);
        let log_file_path = instance_data_dir.join("picodata.log");

        fs::create_dir_all(&instance_data_dir).context("Failed to create instance data dir")?;

        let mut child = Command::new(&run_params.picodata_path);
        child.args([
            "run",
            "--data-dir",
            instance_data_dir.to_str().expect("unreachable"),
            "--plugin-dir",
            plugins_dir.to_str().unwrap_or("target/debug"),
            "--listen",
            &format!("127.0.0.1:{bin_port}"),
            "--peer",
            &format!("127.0.0.1:{first_instance_bin_port}"),
            "--init-replication-factor",
            &replication_factor.to_string(),
            "--http-listen",
            &format!("127.0.0.1:{http_port}"),
            "--pg-listen",
            &format!("127.0.0.1:{pg_port}"),
            "--tier",
            tier,
        ]);

        if run_params.daemon {
            child.stdout(Stdio::null()).stderr(Stdio::null());
            child.args(["--log", log_file_path.to_str().expect("unreachable")]);
        } else {
            child.stdout(Stdio::piped()).stderr(Stdio::piped());
        };

        let child = child
            .spawn()
            .context(format!("failed to start picodata instance: {instance_id}"))?;

        let mut pico_instance = PicodataInstance {
            instance_name,
            tier: tier.to_owned(),
            log_threads: None,
            child,
            daemon: run_params.daemon,
            disable_colors: run_params.disable_colors,
            data_dir: instance_data_dir,
            log_file_path,
        };

        if !run_params.daemon {
            pico_instance.capture_logs()?;
        }

        // Save pid of picodata process to kill it after
        pico_instance.make_pid_file()?;

        Ok(pico_instance)
    }

    fn capture_logs(&mut self) -> Result<()> {
        let mut rnd = rand::rng();
        let instance_name_color = colored::CustomColor::new(
            rnd.random_range(30..220),
            rnd.random_range(30..220),
            rnd.random_range(30..220),
        );

        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.log_file_path)
            .expect("Failed to open log file");
        let file = Arc::new(Mutex::new(file));

        let mut log_threads = vec![];

        let stdout = self.child.stdout.take().expect("Failed to capture stdout");
        let stderr = self.child.stderr.take().expect("Failed to capture stderr");
        let outputs: [Box<dyn Read + Send>; 2] = [Box::new(stdout), Box::new(stderr)];
        for child_output in outputs {
            let mut log_prefix = format!("{}-{}: ", self.tier, self.instance_name);
            if !self.disable_colors {
                log_prefix = log_prefix.custom_color(instance_name_color).to_string();
            }
            let file = file.clone();

            let wrapper = move || {
                let stdout_lines = BufReader::new(child_output).lines();
                for line in stdout_lines {
                    let line = line.unwrap();
                    println!("{log_prefix}{line}");
                    writeln!(file.lock().unwrap(), "{line}")
                        .expect("Failed to write line to log file");
                }
            };

            let t = thread::Builder::new()
                .name(format!("log_catcher::{}", self.instance_name))
                .spawn(wrapper)?;

            log_threads.push(t);
        }

        self.log_threads = Some(log_threads);

        Ok(())
    }

    fn make_pid_file(&self) -> Result<()> {
        let pid = self.child.id();
        let pid_location = self.data_dir.join("pid");
        let mut file = File::create(pid_location)?;
        writeln!(file, "{pid}")?;
        Ok(())
    }

    fn kill(&mut self) -> Result<()> {
        Ok(self.child.kill()?)
    }

    fn join(&mut self) {
        let Some(threads) = self.log_threads.take() else {
            return;
        };
        for h in threads {
            h.join()
                .expect("Failed to join thread for picodata instance");
        }
    }
}

impl Drop for PicodataInstance {
    fn drop(&mut self) {
        if self.daemon {
            return;
        }

        self.child
            .wait()
            .expect("Failed to wait for picodata instance");
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default = "PathBuf::from(\"topology.toml\")")]
    topology_path: PathBuf,
    #[builder(default = "PathBuf::from(\"./tmp\")")]
    data_dir: PathBuf,
    #[builder(default = "false")]
    disable_plugin_install: bool,
    #[builder(default = "8000")]
    base_http_port: u16,
    #[builder(default = "PathBuf::from(\"picodata\")")]
    picodata_path: PathBuf,
    #[builder(default = "5432")]
    base_pg_port: u16,
    #[builder(default = "false")]
    use_release: bool,
    #[builder(default = "PathBuf::from(\"target\")")]
    target_dir: PathBuf,
    #[builder(default = "false")]
    daemon: bool,
    #[builder(default = "false")]
    disable_colors: bool,
}

pub fn cluster(params: &Params) -> Result<Vec<PicodataInstance>> {
    let mut topology: Topology = toml::from_str(
        &fs::read_to_string(&params.topology_path)
            .context(format!("failed to read {}", params.topology_path.display()))?,
    )
    .context(format!(
        "failed to parse .toml file of {}",
        params.topology_path.display()
    ))?;

    let plugins_dir = if params.use_release {
        cargo_build(lib::BuildType::Release)?;
        params.target_dir.join("release")
    } else {
        cargo_build(lib::BuildType::Debug)?;
        params.target_dir.join("debug")
    };

    topology.find_plugin_versions(&plugins_dir)?;

    info!("Running the cluster...");

    let mut picodata_processes = vec![];

    let first_instance_bin_port = 3001;
    let mut instance_id = 0;
    for (tier_name, tier) in &topology.tiers {
        for _ in 0..(tier.replicasets * tier.replication_factor) {
            instance_id += 1;
            let pico_instance = PicodataInstance::new(
                instance_id,
                3000 + instance_id,
                params.base_http_port + instance_id,
                params.base_pg_port + instance_id,
                first_instance_bin_port,
                &plugins_dir,
                tier.replication_factor,
                tier_name,
                params,
            )?;

            picodata_processes.push(pico_instance);

            // TODO: check is started by logs or iproto
            thread::sleep(Duration::from_secs(5));

            info!("i{instance_id} - started");
        }
    }

    if !params.disable_plugin_install {
        info!("Enabling plugins...");

        let result = enable_plugins(&topology, &params.data_dir, &params.picodata_path);
        if let Err(e) = result {
            for process in &mut picodata_processes {
                process.kill().unwrap_or_else(|e| {
                    error!("failed to kill picodata instances: {:#}", e);
                });
            }
            return Err(e.context("failed to enable plugins"));
        }
    };

    info!("Picodata cluster is started");

    Ok(picodata_processes)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
pub fn cmd(params: &Params) -> Result<()> {
    let pico_instances = Arc::new(Mutex::new(cluster(params)?));

    if params.daemon {
        return Ok(());
    }

    // Run in the loop until the child processes are killed
    // with cargo stop or Ctrl+C signal is recieved
    let picodata_processes = pico_instances.clone();
    ctrlc::set_handler(move || {
        info!("received Ctrl+C. Shutting down ...");

        let mut childs = picodata_processes.lock().unwrap();
        for process in childs.iter_mut() {
            process.kill().unwrap_or_else(|e| {
                error!("failed to kill picodata instances: {:#}", e);
            });
        }

        exit(0);
    })
    .context("failed to set Ctrl+c handler")?;

    for instance in pico_instances.lock().unwrap().iter_mut() {
        instance.join();
    }

    Ok(())
}
