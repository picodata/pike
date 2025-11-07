use anyhow::{bail, Context, Result};
use derive_builder::Builder;
use log::info;
use serde::Deserialize;
use std::{
    collections::HashMap,
    env, fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
};

/// Mapping of plugin service names to their properties specified in
/// [plugin configuration](https://github.com/picodata/pike?tab=readme-ov-file#config-apply).
///
/// ### Example:
///
/// Assume plugin configuration YAML has the following content:
///
/// **`plugin_config.yaml`:**
/// ```yaml
/// service_name:
///   http_server:
///     url: "www.example.com"
/// ```
/// Mapping for such config is supposed to look like:
///
/// ```rust,no_run
/// use std::collections::HashMap;
///
/// let plugin_config = HashMap::from([(
///     // Name of the service
///     "service_name".to_string(),
///
///     // Mapping of properties corresponding to the service
///     HashMap::from([(
///         "http_server".to_string(),
///         serde_yaml::to_value(HashMap::from([(
///             "url".to_string(),
///             // URL is overridden for testing.
///             "localhost:29092".to_string(),
///         )]))
///         .unwrap(),
///     )]),
/// )]);
/// ```
pub type ConfigMap = HashMap<String, HashMap<String, serde_yaml::Value>>;

const DEFAULT_PLUGIN_CONFIG_PATH: &str = "plugin_config.yaml";
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

fn read_config_from_path(path: &PathBuf) -> Result<ConfigMap> {
    serde_yaml::from_str(
        &fs::read_to_string(path)
            .context(format!("failed to read config file at {}", path.display()))?,
    )
    .context(format!(
        "failed to parse config file at {} as toml",
        path.display()
    ))
}

fn apply_service_config(
    plugin_name: &str,
    plugin_version: &str,
    service_name: &str,
    config: &HashMap<String, serde_yaml::Value>,
    admin_socket: &Path,
    picodata_path: &Path,
) -> Result<()> {
    let mut queries: Vec<String> = Vec::new();

    for (key, value) in config {
        let value = serde_json::to_string(&value)
            .context(format!("failed to serialize the string with key {key}"))?;
        queries.push(format!(
            r#"ALTER PLUGIN "{plugin_name}" {plugin_version} SET "{service_name}"."{key}"='{value}';"#
        ));
    }

    for query in queries {
        log::info!("picodata admin: {query}");

        let mut picodata_admin = Command::new(picodata_path)
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

        let exit_status = picodata_admin
            .wait()
            .context("failed to wait for picodata admin")?
            .code()
            .unwrap();

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

        if exit_status == 1 {
            bail!("failed to execute picodata query {query}");
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

    let config: ConfigMap = match &params.config_source {
        ConfigSource::Map(map) => map.clone(),
        ConfigSource::Path(path) => read_config_from_path(&cur_plugin_dir.join(path))?,
    };

    for (service_name, service_config) in config {
        apply_service_config(
            &cargo_manifest.package.name,
            &cargo_manifest.package.version,
            &service_name,
            &service_config,
            &admin_socket,
            &params.picodata_path,
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

#[derive(Debug, Clone)]
pub enum ConfigSource {
    Map(ConfigMap),
    Path(PathBuf),
}

impl Default for ConfigSource {
    fn default() -> Self {
        ConfigSource::Path(DEFAULT_PLUGIN_CONFIG_PATH.into())
    }
}

#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default, setter(custom))]
    config_source: ConfigSource,
    #[builder(default = "PathBuf::from(\"./tmp\")")]
    data_dir: PathBuf,
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default)]
    plugin_name: Option<String>,
    #[builder(default = "PathBuf::from(\"picodata\")")]
    picodata_path: PathBuf,
}

impl ParamsBuilder {
    pub fn config_path(&mut self, path: PathBuf) -> &mut Self {
        self.config_source = Some(ConfigSource::Path(path));
        self
    }

    #[allow(unused)]
    pub fn config_map(&mut self, map: ConfigMap) -> &mut Self {
        self.config_source = Some(ConfigSource::Map(map));
        self
    }
}

pub fn cmd(params: &Params) -> Result<()> {
    // If plugin name flag was specified, apply config only for
    // this exact plugin
    if let Some(plugin_name) = &params.plugin_name {
        info!("Applying plugin config for plugin {plugin_name}");
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
        if let ConfigSource::Path(config_path) = &params.config_source {
            if config_path.to_str().unwrap() != DEFAULT_PLUGIN_CONFIG_PATH {
                println!("{WISE_PIKE}");
                process::exit(1);
            }
        }
        info!("Applying plugin config for each plugin");

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(prefix: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = env::temp_dir();
        dir.push(format!("pike-config-apply-ut-{prefix}-{ts}"));
        dir
    }

    #[test]
    fn apply_service_config_uses_custom_picodata_path_and_fails_cleanly() {
        let mut service_cfg: HashMap<String, serde_yaml::Value> = HashMap::new();
        service_cfg.insert(
            "k".to_string(),
            serde_yaml::to_value("v").expect("serialize test value"),
        );

        let bogus_picodata = PathBuf::from("/this/does/not/exist/picodata-bogus");
        let bogus_socket = Path::new("/tmp/nonexistent-admin.sock");

        let err = apply_service_config(
            "p",
            "0.1.0",
            "svc",
            &service_cfg,
            bogus_socket,
            &bogus_picodata,
        )
        .unwrap_err();

        let msg = format!("{err:#}");
        assert!(
            msg.contains("failed to run picodata admin"),
            "expected process spawn error context, got: {msg}"
        );
    }

    #[test]
    fn params_builder_has_default_picodata_path() {
        let params = ParamsBuilder::default().build().unwrap();
        assert_eq!(params.picodata_path, PathBuf::from("picodata"));
    }

    #[test]
    fn read_config_from_path_reports_read_error() {
        let dir = tmp_dir("cfg");
        let cfg = dir.join("no-file.yaml");
        let res = read_config_from_path(&cfg);
        assert!(res.is_err());
    }
}
