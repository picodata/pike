use anyhow::{bail, Context, Result};
use lib::cargo_build;
use log::{error, info};
use serde::Deserialize;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{exit, Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use std::{collections::HashMap, fs, path::PathBuf};

use crate::commands::lib;

#[derive(Debug, Deserialize)]
struct Service {
    name: String,
    plugin: String,
}

#[derive(Debug, Deserialize)]
struct MigrationEnv {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct Tier {
    instances: u8,
    replication_factor: u8,
    migration_envs: Option<Vec<MigrationEnv>>,
    services: Option<Vec<Service>>,
}

#[derive(Debug, Deserialize)]
struct Topology {
    tiers: HashMap<String, Tier>,
}

static PICODATA_PROCESSES: OnceLock<Arc<Mutex<Vec<Child>>>> = OnceLock::new();

fn get_picodata_processes() -> Arc<Mutex<Vec<Child>>> {
    PICODATA_PROCESSES
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

fn enable_plugins(
    topology: &Topology,
    data_dir: &Path,
    picodata_path: &PathBuf,
    plugins_dir: &Path,
) -> Result<()> {
    let mut plugins: HashMap<String, String> = HashMap::new();
    for tier in topology.tiers.values() {
        let Some(services) = &tier.services else {
            continue;
        };
        for service in services {
            let current_plugin_dir = plugins_dir.join(service.plugin.clone());

            if !current_plugin_dir.exists() {
                bail!(
                    "directory {} does not exist, run \"cargo build\" inside plugin directory",
                    current_plugin_dir.display()
                );
            }
            plugins.entry(service.plugin.clone()).or_insert_with(|| {
                let mut versions: Vec<_> = fs::read_dir(current_plugin_dir)
                    .unwrap()
                    .map(|r| r.unwrap())
                    .collect();
                versions.sort_by_key(std::fs::DirEntry::path);
                versions
                    .last()
                    .unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .to_string()
            });
        }
    }

    let mut queries: Vec<String> = Vec::new();
    // Queries to set migration variables, order of commands is not important
    push_migration_envs_queries(&mut queries, topology, &plugins);

    for (plugin, version) in &plugins {
        queries.push(format!(r#"CREATE PLUGIN "{plugin}" {version};"#));
        queries.push(format!(r#"ALTER PLUGIN "{plugin}" MIGRATE TO {version};"#));
        queries.push(format!(r#"ALTER PLUGIN "{plugin}" {version} ENABLE;"#));
    }

    for (tier_name, tier) in &topology.tiers {
        let Some(services) = &tier.services else {
            continue;
        };
        for service in services {
            let plugin_name = &service.plugin;
            let plugin_version = plugins
                .get(plugin_name)
                .context("failed to find plugin version")?;
            let service_name = &service.name;
            queries.push(format!(r#"ALTER PLUGIN "{plugin_name}" {plugin_version} ADD SERVICE "{service_name}" TO TIER "{tier_name}";"#));
        }
    }

    let admin_soket = data_dir.join("cluster").join("i_1").join("admin.sock");

    for query in queries {
        let mut picodata_admin = Command::new(picodata_path)
            .arg("admin")
            .arg(admin_soket.to_str().unwrap())
            .stdin(Stdio::piped())
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

        thread::sleep(Duration::from_secs(3));
    }

    Ok(())
}

fn push_migration_envs_queries(
    queries: &mut Vec<String>,
    topology: &Topology,
    plugins: &HashMap<String, String>,
) {
    info!("setting migration variables");

    for tier in topology.tiers.values() {
        let Some(migration_envs) = &tier.migration_envs else {
            continue;
        };
        for migration_env in migration_envs {
            for (plugin, version) in plugins {
                queries.push(format!(
                    r#"ALTER PLUGIN {plugin} {version} SET migration_context.{}='{}';"#,
                    migration_env.name, migration_env.value
                ));
            }
        }
    }
}

fn kill_picodata_instances() -> Result<()> {
    let processes_lock = Arc::clone(&get_picodata_processes());
    let mut processes = processes_lock.lock().unwrap();

    for mut process in processes.drain(..) {
        process.kill()?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn cluster(
    topology_path: &PathBuf,
    data_dir: &Path,
    disable_plugin_install: bool,
    base_http_port: i32,
    picodata_path: &PathBuf,
    base_pg_port: i32,
    use_release: bool,
    target_dir: &Path,
) -> Result<()> {
    let topology: &Topology = &toml::from_str(
        &fs::read_to_string(topology_path)
            .context(format!("failed to read {}", topology_path.display()))?,
    )
    .context(format!(
        "failed to parse .toml file of {}",
        topology_path.display()
    ))?;

    let plugins_dir = if use_release {
        cargo_build(lib::BuildType::Release)?;
        target_dir.join("release")
    } else {
        cargo_build(lib::BuildType::Debug)?;
        target_dir.join("debug")
    };

    let first_instance_bin_port = 3001;
    let mut instance_id = 0;
    for (tier_name, tier) in &topology.tiers {
        for _ in 0..(tier.instances * tier.replication_factor) {
            instance_id += 1;
            let bin_port = 3000 + instance_id;
            let http_port = base_http_port + instance_id;
            let pg_port = base_pg_port + instance_id;
            let instance_data_dir = data_dir.join("cluster").join(format!("i_{instance_id}"));

            // TODO: make it as child processes with catch output and redirect it to main
            // output
            let process = Command::new(picodata_path)
                .args([
                    "run",
                    "--data-dir",
                    instance_data_dir
                        .to_str()
                        .context("Invalid data dir path")?,
                    "--plugin-dir",
                    plugins_dir.to_str().unwrap_or("target"),
                    "--listen",
                    &format!("127.0.0.1:{bin_port}"),
                    "--peer",
                    &format!("127.0.0.1:{first_instance_bin_port}"),
                    "--init-replication-factor",
                    &tier.replication_factor.to_string(),
                    "--http-listen",
                    &format!("127.0.0.1:{http_port}"),
                    "--pg-listen",
                    &format!("127.0.0.1:{pg_port}"),
                    "--tier",
                    tier_name,
                ])
                .spawn()
                .context(format!("failed to start picodata instance: {instance_id}"))?;
            thread::sleep(Duration::from_secs(5));

            // Save pid of picodata process to kill it after
            let pid = process.id();
            let pid_location = instance_data_dir.join("pid");
            let mut file = File::create(pid_location)?;
            writeln!(file, "{pid}")?;

            let processes_lock = Arc::clone(&get_picodata_processes());
            let mut processes = processes_lock.lock().unwrap();
            processes.push(process);
        }
    }

    if !disable_plugin_install {
        enable_plugins(topology, data_dir, picodata_path, &plugins_dir)
            .inspect_err(|_| {
                kill_picodata_instances().unwrap_or_else(|e| {
                    error!("failed to kill picodata instances: {:#}", e);
                });
            })
            .context("failed to enable plugins")?;
    };

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn cmd(
    topology_path: &PathBuf,
    data_dir: &Path,
    disable_plugin_install: bool,
    base_http_port: i32,
    picodata_path: &PathBuf,
    base_pg_port: i32,
    use_release: bool,
    target_dir: &Path,
    daemon: bool,
) -> Result<()> {
    fs::create_dir_all(data_dir).unwrap();

    if !daemon {
        ctrlc::set_handler(move || {
            info!("{}", "received Ctrl+C. Shutting down ...");

            kill_picodata_instances()
                .unwrap_or_else(|e| error!("failed to kill picodata instances: {:#}", e));

            exit(0);
        })
        .context("failed to set Ctrl+c handler")?;
    }

    cluster(
        topology_path,
        data_dir,
        disable_plugin_install,
        base_http_port,
        picodata_path,
        base_pg_port,
        use_release,
        target_dir,
    )?;

    // Run in the loop until the child processes are killed
    // with cargo stop or Ctrl+C signal is recieved
    if !daemon {
        loop {
            thread::sleep(std::time::Duration::from_millis(100));
            let processes_lock = Arc::clone(&get_picodata_processes());
            let mut processes = processes_lock.lock().unwrap();

            let all_proccesses_ended = processes
                .iter_mut()
                .all(|p| p.try_wait().unwrap().is_some());

            if all_proccesses_ended {
                info!("{}", "all child processes have ended, shutting down...");
                break;
            }
        }
    }

    Ok(())
}
