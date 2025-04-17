use anyhow::{bail, Context, Result};
use colored::Colorize;
use derive_builder::Builder;
use lib::cargo_build;
use log::{error, info};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use rand::Rng;
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::str;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use crate::commands::lib;

#[derive(Debug, Deserialize, Clone)]
pub struct Tier {
    pub replicasets: u8,
    pub replication_factor: u8,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MigrationContextVar {
    pub name: String,
    pub value: String,
}

#[derive(Default, Debug, Deserialize, Clone)]
pub struct Service {
    pub tiers: Vec<String>,
}

#[derive(Default, Debug, Deserialize, Clone)]
pub struct Plugin {
    #[serde(default)]
    pub migration_context: Vec<MigrationContextVar>,
    #[serde(default)]
    #[serde(rename = "service")]
    pub services: BTreeMap<String, Service>,
    #[serde(skip)]
    pub version: Option<String>,
}

#[derive(Default, Debug, Deserialize, Clone)]
pub struct Topology {
    #[serde(rename = "tier")]
    pub tiers: BTreeMap<String, Tier>,
    #[serde(rename = "plugin")]
    #[serde(default)]
    pub plugins: BTreeMap<String, Plugin>,
    #[serde(default)]
    pub enviroment: BTreeMap<String, String>,
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
                "ALTER PLUGIN \"{plugin_name}\" {plugin_version} SET migration_context.{}='{}';",
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
            "Plugin {plugin_name}:{} has been enabled",
            plugin.version.as_ref().unwrap()
        );
    }

    Ok(())
}

const GET_VERSION_LUA: &str = "\\lua\npico.instance_info().name\n";

fn get_instance_name(picodata_path: &Path, instance_data_dir: &Path) -> Result<String> {
    let admin_soket = instance_data_dir.join("admin.sock");
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
            .write_all(GET_VERSION_LUA.as_bytes())
            .context("failed to send plugin installation queries")?;
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
            .unwrap();
        bail!("get version error: {stderr}");
    }

    let mut stdout = String::new();
    picodata_admin
        .stdout
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();

    for line in stdout.split('\n') {
        if line.starts_with("- ") {
            return Ok(line.strip_prefix("- ").unwrap().to_string());
        }
    }

    bail!("get version error: {stdout}");
}

fn is_plugin_dir(path: &Path) -> bool {
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

pub struct PicodataInstance {
    instance_name: String,
    log_threads: Option<Vec<JoinHandle<()>>>,
    child: Child,
    daemon: bool,
    disable_colors: bool,
    data_dir: PathBuf,
    log_file_path: PathBuf,
    pg_port: u16,
}

impl PicodataInstance {
    #[allow(clippy::too_many_arguments)]
    fn new(
        instance_id: u16,
        bin_port: u16,
        http_port: u16,
        pg_port: u16,
        first_instance_bin_port: u16,
        plugins_dir: Option<&Path>,
        tier: &str,
        run_params: &Params,
        env_templates: &BTreeMap<String, String>,
        tiers_config: &str,
        picodata_path: &Path,
    ) -> Result<Self> {
        let mut instance_name = format!("i{instance_id}");
        let instance_data_dir = run_params.data_dir.join("cluster").join(&instance_name);
        let log_file_path = instance_data_dir.join("picodata.log");

        fs::create_dir_all(&instance_data_dir).context("Failed to create instance data dir")?;

        let env_templates_ctx = liquid::object!({
            "instance_id": instance_id,
        });
        let env_vars = Self::compute_env_vars(env_templates, &env_templates_ctx)?;

        let mut child = Command::new(&run_params.picodata_path);
        child.envs(&env_vars);

        let picodata_version = Self::get_picodata_version(&run_params.picodata_path)?;
        let data_dir_flag = if picodata_version.contains("picodata 24.6") {
            log::warn!(
                "You are using old version of picodata: {picodata_version} In the next major release it WILL NOT BE SUPPORTED"
            );
            "--data-dir"
        } else {
            "--instance-dir"
        };

        let listen_flag = if picodata_version.contains("picodata 24.6") {
            "--listen"
        } else {
            "--iproto-listen"
        };

        child.args([
            "run",
            data_dir_flag,
            instance_data_dir.to_str().expect("unreachable"),
            listen_flag,
            &format!("127.0.0.1:{bin_port}"),
            "--peer",
            &format!("127.0.0.1:{first_instance_bin_port}"),
            "--http-listen",
            &format!("0.0.0.0:{http_port}"),
            "--pg-listen",
            &format!("127.0.0.1:{pg_port}"),
            "--tier",
            tier,
            "--config-parameter",
            &format!("cluster.tier={tiers_config}",),
        ]);

        if let Some(plugins_dir) = plugins_dir {
            child.args([
                "--plugin-dir",
                plugins_dir.to_str().unwrap_or("target/debug"),
            ]);
        }

        if run_params.daemon {
            child.stdout(Stdio::null()).stderr(Stdio::null());
            child.args(["--log", log_file_path.to_str().expect("unreachable")]);
        } else {
            child.stdout(Stdio::piped()).stderr(Stdio::piped());
        };

        let child = child
            .spawn()
            .context(format!("failed to start picodata instance: {instance_id}"))?;

        let start = Instant::now();
        while Instant::now().duration_since(start) < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(100));
            let new_instance_name = match get_instance_name(picodata_path, &instance_data_dir) {
                Ok(name) => name,
                Err(e) => {
                    log::debug!("{e}");
                    continue;
                }
            };

            // create symlink to real instance data dir
            let symlink_name = run_params.data_dir.join("cluster").join(&new_instance_name);
            let _ = fs::remove_file(&symlink_name);
            symlink(&instance_name, symlink_name)
                .context("failed create symlink to instance dir")?;

            instance_name = new_instance_name;
            break;
        }

        let mut pico_instance = PicodataInstance {
            instance_name,
            log_threads: None,
            child,
            daemon: run_params.daemon,
            disable_colors: run_params.disable_colors,
            data_dir: instance_data_dir,
            log_file_path,
            pg_port,
        };

        if !run_params.daemon {
            pico_instance.capture_logs()?;
        }

        // Save pid of picodata process to kill it after
        pico_instance.make_pid_file()?;

        Ok(pico_instance)
    }

    fn get_picodata_version(picodata_path: &PathBuf) -> Result<String> {
        let picodata_output = Command::new(picodata_path)
            .arg("--version")
            .output()
            .expect("Failed to execute command");

        Ok(str::from_utf8(&picodata_output.stdout)?.to_string())
    }

    #[allow(dead_code)]
    #[allow(clippy::must_use_candidate)]
    pub fn pg_port(&self) -> &u16 {
        &self.pg_port
    }

    fn compute_env_vars(
        env_templates: &BTreeMap<String, String>,
        ctx: &liquid::Object,
    ) -> Result<BTreeMap<String, String>> {
        env_templates
            .iter()
            .map(|(k, v)| {
                let tpl = liquid::ParserBuilder::with_stdlib().build()?.parse(v)?;
                Ok((k.clone(), tpl.render(ctx)?))
            })
            .collect()
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
            let mut log_prefix = format!("{}: ", self.instance_name);
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

fn get_merged_cluster_tier_config(plugin_path: &Path, tiers: &BTreeMap<String, Tier>) -> String {
    let picodata_conf_path = plugin_path.join("picodata.yaml");
    let picodata_conf_raw = fs::read_to_string(picodata_conf_path).unwrap_or_default();
    let picodata_conf: HashMap<String, Value> =
        serde_yaml::from_str(&picodata_conf_raw).unwrap_or_default();

    let cluster_params = picodata_conf
        .get("cluster")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_else(Mapping::new);

    let mut tier_params = cluster_params
        .get("tier")
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_else(Mapping::new);

    for value in tier_params.values_mut() {
        if value.is_null() {
            *value = Value::Mapping(Mapping::new());
        }
    }

    for (tier_name, tier_value) in tiers {
        tier_params
            .entry(Value::String(tier_name.clone()))
            .and_modify(|entry| {
                if let Value::Mapping(ref mut map) = entry {
                    map.insert(
                        Value::String("replication_factor".into()),
                        Value::Number(tier_value.replication_factor.into()),
                    );
                }
            })
            .or_insert_with(|| {
                let mut map = Mapping::new();
                map.insert(
                    Value::String("replication_factor".into()),
                    Value::Number(tier_value.replication_factor.into()),
                );
                Value::Mapping(map)
            });
    }

    serde_json::to_string(&tier_params).unwrap()
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Builder, Clone)]
pub struct Params {
    topology: Topology,
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
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default = "false")]
    no_build: bool,
}

pub fn cluster(params: &Params) -> Result<Vec<PicodataInstance>> {
    let mut params = params.clone();
    params.data_dir = params.plugin_path.join(&params.data_dir);

    let mut plugins_dir = None;
    if is_plugin_dir(&params.plugin_path) {
        let build_type = if params.use_release {
            plugins_dir = Some(params.plugin_path.join(params.target_dir.join("release")));
            lib::BuildType::Release
        } else {
            plugins_dir = Some(params.plugin_path.join(params.target_dir.join("debug")));
            lib::BuildType::Debug
        };
        if !params.no_build {
            cargo_build(build_type, &params.target_dir, &params.plugin_path)?;
        };

        params
            .topology
            .find_plugin_versions(plugins_dir.as_ref().unwrap())?;
    }

    info!("Running the cluster...");
    let start_cluster_run = Instant::now();

    let mut picodata_processes = vec![];

    let tiers_config = get_merged_cluster_tier_config(&params.plugin_path, &params.topology.tiers);

    let first_instance_bin_port = 3001;
    let mut instance_id = 0;
    for (tier_name, tier) in &params.topology.tiers {
        for _ in 0..(tier.replicasets * tier.replication_factor) {
            instance_id += 1;
            let pico_instance = PicodataInstance::new(
                instance_id,
                3000 + instance_id,
                params.base_http_port + instance_id,
                params.base_pg_port + instance_id,
                first_instance_bin_port,
                plugins_dir.as_deref(),
                tier_name,
                &params,
                &params.topology.enviroment,
                &tiers_config,
                &params.picodata_path,
            )?;

            picodata_processes.push(pico_instance);

            info!("i{instance_id} - started");
        }
    }

    // TODO: check cluster is started by logs or iproto
    thread::sleep(Duration::from_secs(3));

    if !params.disable_plugin_install {
        info!("Enabling plugins...");

        if plugins_dir.is_some() {
            let result = enable_plugins(&params.topology, &params.data_dir, &params.picodata_path);
            if let Err(e) = result {
                for process in &mut picodata_processes {
                    process.kill().unwrap_or_else(|e| {
                        error!("failed to kill picodata instances: {e:#}");
                    });
                }
                return Err(e.context("failed to enable plugins"));
            }
        }
    };

    info!(
        "Picodata cluster has started (launch time: {} sec, total instances: {instance_id})",
        start_cluster_run.elapsed().as_secs()
    );

    Ok(picodata_processes)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::fn_params_excessive_bools)]
#[allow(clippy::cast_possible_wrap)]
pub fn cmd(params: &Params) -> Result<()> {
    let mut pico_instances = cluster(params)?;

    if params.daemon {
        return Ok(());
    }

    // Set Ctrl+C handler. Upon recieving Ctrl+C signal
    // All instances would be killed, then joined and
    // destructors will be called
    let picodata_pids: Vec<u32> = pico_instances.iter().map(|p| p.child.id()).collect();
    ctrlc::set_handler(move || {
        info!("received Ctrl+C. Shutting down ...");

        for &pid in &picodata_pids {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
    })
    .context("failed to set Ctrl+c handler")?;

    // Wait for all instances to stop
    for instance in &mut pico_instances {
        instance.join();
    }

    Ok(())
}
