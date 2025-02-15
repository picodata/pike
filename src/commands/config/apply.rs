use anyhow::{Context, Result};
use derive_builder::Builder;
use log::info;
use serde::Deserialize;
use serde_yaml::Value;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn apply_service_config(
    plugin_name: &str,
    plugin_version: &str,
    service_name: &str,
    config: &HashMap<String, Value>,
    admin_socket: &Path,
) -> Result<()> {
    let mut queries: Vec<String> = Vec::new();

    for (key, value) in config {
        let value = serde_json::to_string(&value)
            .context(format!("failed to serialize the string with key {key}"))?;
        queries.push(format!(
            r#"ALTER PLUGIN "{plugin_name}" {plugin_version} SET {service_name}.{key}='{value}';"#
        ));
    }

    for query in queries {
        log::info!("picodata admin: {query}");

        let mut picodata_admin = Command::new("picodata")
            .arg("admin")
            .arg(
                admin_socket
                    .to_str()
                    .context("path to picodata admin socket contains invalid characters")?,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .context("failed to run picodata admin")?;

        {
            let picodata_stdin = picodata_admin
                .stdin
                .as_mut()
                .context("failed to get picodata stdin")?;
            picodata_stdin
                .write_all(query.as_bytes())
                .context("failed to push queries into picodata admin")?;
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

    Ok(())
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct CargoManifest {
    package: Package,
}

#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default = "PathBuf::from(\"plugin_config.yaml\")")]
    config_path: PathBuf,
    #[builder(default = "PathBuf::from(\"./tmp\")")]
    data_dir: PathBuf,
}

pub fn cmd(params: &Params) -> Result<()> {
    info!("Applying plugin config...");

    let admin_socket = params
        .data_dir
        .join("cluster")
        .join("i1")
        .join("admin.sock");
    let cargo_manifest: &CargoManifest =
        &toml::from_str(&fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?)
            .context("failed to parse Cargo.toml")?;
    let config: HashMap<String, HashMap<String, Value>> =
        serde_yaml::from_str(&fs::read_to_string(&params.config_path).context(format!(
            "failed to read config file at {}",
            params.config_path.display()
        ))?)
        .context(format!(
            "failed to parse config file at {} as toml",
            params.config_path.display()
        ))?;
    for (service_name, service_config) in config {
        apply_service_config(
            &cargo_manifest.package.name,
            &cargo_manifest.package.version,
            &service_name,
            &service_config,
            &admin_socket,
        )
        .context(format!(
            "failed to apply service config for service {service_name}"
        ))?;
    }

    info!("Plugin config successfully applied.");

    Ok(())
}
