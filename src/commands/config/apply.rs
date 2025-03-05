use anyhow::{Context, Result};
use derive_builder::Builder;
use log::info;
use serde::Deserialize;
use serde_yaml::Value;
use std::{
    collections::HashMap,
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
};

const WISE_PIKE: &str = r"
  ________________________________________
/ You are trying to apply config from     \
| custom directory, however to use this   |
| flag, you must specify the plugin with  |
\           --plugin-name                 /
 ----------------------------------------
 o
o      ______/~/~/~/__           /((
  o  // __            ====__    /_((
 o  //  @))       ))))      ===/__((
    ))           )))))))        __((
    \\     \)     ))))    __===\ _((
     \\_______________====      \_((
                                 \((
 ";

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

fn apply_plugin_config(params: &Params, current_plugin_path: &str) -> Result<()> {
    let cur_plugin_dir = env::current_dir()?
        .join(&params.plugin_path)
        .join(current_plugin_path);

    let admin_socket = params
        .plugin_path
        .join(&params.data_dir)
        .join("cluster")
        .join("i1")
        .join("admin.sock");

    let cargo_manifest: &CargoManifest = &toml::from_str(
        &fs::read_to_string(cur_plugin_dir.join("Cargo.toml"))
            .context("failed to read Cargo.toml")?,
    )
    .context("failed to parse Cargo.toml")?;

    let config: HashMap<String, HashMap<String, Value>> = serde_yaml::from_str(
        &fs::read_to_string(cur_plugin_dir.join(&params.config_path)).context(format!(
            "failed to read config file at {}",
            params.config_path.display()
        ))?,
    )
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
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    plugin_name: Option<String>,
}

pub fn cmd(params: &Params) -> Result<()> {
    // If plugin name flag was specified, apply config only for
    // this exact plugin
    if let Some(plugin_name) = &params.plugin_name {
        info!("Applying plugin config for plugin {}", plugin_name);
        apply_plugin_config(params, plugin_name)?;
        return Ok(());
    }

    let root_dir = env::current_dir()?.join(&params.plugin_path);

    let cargo_toml_path = root_dir.join("Cargo.toml");
    let cargo_toml_content = fs::read_to_string(&cargo_toml_path).context(format!(
        "Failed to read Cargo.toml in {}",
        &cargo_toml_path.display()
    ))?;

    let parsed_toml: toml::Value = cargo_toml_content
        .parse()
        .context("Failed to parse Cargo.toml")?;

    if let Some(workspace) = parsed_toml.get("workspace") {
        if params.config_path.to_str().unwrap() != "plugin_config.yaml" {
            println!("{WISE_PIKE}");
            process::exit(1);
        }
        info!("Applying plugin config in each plugin");

        if let Some(members) = workspace.get("members") {
            if let Some(members_array) = members.as_array() {
                for member in members_array {
                    let member_str = member.as_str();
                    if member_str.is_none() {
                        continue;
                    }

                    if !root_dir
                        .join(member_str.unwrap())
                        .join("manifest.yaml.template")
                        .exists()
                    {
                        continue;
                    }
                    apply_plugin_config(params, member_str.unwrap())?;
                }
            }
        }

        return Ok(());
    }

    info!("Applying plugin config");

    apply_plugin_config(params, "./")?;

    Ok(())
}
