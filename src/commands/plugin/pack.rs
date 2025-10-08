use crate::commands::lib::{cargo_build, BuildType, LIB_EXT};
use anyhow::{anyhow, bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{debug, info, warn};
use serde::Deserialize;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::{env, fs};
use tar::Builder;
use toml::Value;

#[derive(Deserialize)]
struct PackageInfo {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct CargoManifest {
    package: PackageInfo,
}

pub fn cmd(pack_debug: bool, target_dir: &PathBuf, plugin_path: &PathBuf) -> Result<()> {
    let current_dir = env::current_dir().context("failed to get current working directory")?;
    let root_dir = if plugin_path.is_absolute() {
        plugin_path.clone()
    } else {
        current_dir.join(plugin_path)
    };

    if !root_dir.join("Cargo.toml").exists() {
        bail!("No Cargo.toml found at plugin path: {}", root_dir.display());
    }

    let build_type = if pack_debug {
        BuildType::Debug
    } else {
        BuildType::Release
    };

    cargo_build(build_type, target_dir, plugin_path)
        .with_context(|| format!("building {build_type} version of plugin"))?;

    let build_root = {
        let effective_target_dir = if target_dir.is_absolute() {
            target_dir.clone()
        } else {
            root_dir.join(target_dir)
        };
        effective_target_dir.join(build_type.to_string())
    };

    let cargo_toml_path = root_dir.join("Cargo.toml");
    let cargo_toml_content = fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("Failed to read Cargo.toml in {}", cargo_toml_path.display()))?;

    let parsed_toml: Value = cargo_toml_content
        .parse()
        .context("Failed to parse Cargo.toml")?;

    if let Some(workspace) = parsed_toml.get("workspace") {
        let mut packaged_any = false;
        if let Some(members) = workspace.get("members").and_then(|m| m.as_array()) {
            for member in members {
                let Some(member_str) = member.as_str() else {
                    warn!("Skipping non-string workspace member entry: {member:?}");
                    continue;
                };
                let member_path = root_dir.join(member_str);
                if member_path.join("manifest.yaml.template").exists() {
                    info!("Packing workspace member plugin: {}", member_path.display());
                    create_plugin_archive(&build_root, &member_path)?;
                    packaged_any = true;
                } else {
                    debug!(
                        "Workspace member {} has no manifest.yaml.template — skipping",
                        member_path.display()
                    );
                }
            }
        }
        if !packaged_any {
            warn!(
                "No workspace members produced plugin archives (no manifest.yaml.template found)."
            );
        }
        return Ok(());
    }

    create_plugin_archive(&build_root, &root_dir)
}

fn create_plugin_archive(build_dir: &Path, plugin_dir: &Path) -> Result<()> {
    let plugin_version = get_latest_plugin_version(plugin_dir)?;
    let cargo_manifest: CargoManifest = toml::from_str(
        &fs::read_to_string(plugin_dir.join("Cargo.toml"))
            .context("failed to read Cargo.toml for packaging")?,
    )
    .context("failed to parse Cargo.toml for packaging")?;

    let package_name = cargo_manifest.package.name;
    let normalized_package_name = package_name.replace('-', "_");
    let plugin_build_dir = build_dir.join(&package_name).join(&plugin_version);
    let root_in_archive = Path::new(&package_name).join(&plugin_version);

    let os_suffix = detect_os_suffix().context("failed to detect OS for archive naming")?;

    let archive_filename = format!(
        "{}_{}-{}.tar.gz",
        package_name, cargo_manifest.package.version, os_suffix
    );
    let compressed_file_path = build_dir.join(&archive_filename);

    if !plugin_build_dir.exists() {
        bail!(
            "Expected build output directory not found: {}",
            plugin_build_dir.display()
        );
    }

    info!(
        "Packing plugin '{}' (version {}) → {}",
        package_name,
        plugin_version,
        compressed_file_path.display()
    );

    let compressed_file =
        File::create(&compressed_file_path).context("failed to create archive file")?;
    let mut encoder = GzEncoder::new(compressed_file, Compression::best());

    {
        let mut tarball = Builder::new(&mut encoder);

        let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");
        archive_if_exists(
            &root_in_archive,
            &plugin_build_dir.join(&lib_name),
            &mut tarball,
        )?;
        archive_if_exists(
            &root_in_archive,
            &plugin_build_dir.join("manifest.yaml"),
            &mut tarball,
        )?;
        archive_if_exists(
            &root_in_archive,
            &plugin_build_dir.join("migrations"),
            &mut tarball,
        )?;

        let assets_dir = plugin_build_dir.join("assets");
        if assets_dir.exists() {
            for entry in fs::read_dir(&assets_dir)
                .with_context(|| format!("reading assets dir {}", assets_dir.display()))?
            {
                let entry = entry?;
                archive_if_exists(
                    &root_in_archive,
                    &assets_dir.join(entry.file_name()),
                    &mut tarball,
                )?;
            }
        }

        tarball
            .finish()
            .context("failed to finish building tar archive")?;
    }

    encoder
        .try_finish()
        .context("failed to finish compression")?;

    info!("Archive created: {}", compressed_file_path.display());
    Ok(())
}

// ---------------- OS detection (per target) ----------------

#[cfg(target_os = "linux")]
fn detect_os_suffix() -> Result<String> {
    detect_linux_os_suffix()
}

#[cfg(target_os = "macos")]
fn detect_os_suffix() -> Result<String> {
    detect_macos_os_suffix()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_os_suffix() -> Result<String> {
    bail!("unsupported operating system for packing plugin (supported: linux, macos)");
}

#[cfg(target_os = "linux")]
fn detect_linux_os_suffix() -> Result<String> {
    const OS_RELEASE: &str = "/etc/os-release";
    const ROLLING_DISTROS: &[&str] = &[
        "arch",
        "gentoo",
        "void",
        "opensuse-tumbleweed",
        "artix",
        "manjaro",
        "endeavouros",
        "garuda",
        "kaos",
    ];

    let content = fs::read_to_string(OS_RELEASE)
        .with_context(|| format!("failed to read {OS_RELEASE} for determining OS"))?;

    let mut id: Option<String> = None;
    let mut version_id: Option<String> = None;
    let mut version_codename: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let mut val = v.trim().trim_matches('"').trim_matches('\'').to_string();
            if key.eq_ignore_ascii_case("ID") {
                id = Some(val.to_ascii_lowercase());
            } else if key.eq_ignore_ascii_case("VERSION_ID") {
                val = val.replace(' ', "_").to_ascii_lowercase();
                version_id = Some(val);
            } else if key.eq_ignore_ascii_case("VERSION_CODENAME") {
                version_codename = Some(val.to_ascii_lowercase());
            }
        }
    }

    let id = id.ok_or_else(|| anyhow!("ID not found in /etc/os-release"))?;

    let variant = if let Some(vid) = version_id {
        if vid.is_empty() {
            resolve_linux_variant(&id, ROLLING_DISTROS)
        } else {
            vid
        }
    } else if let Some(code) = version_codename {
        if code.is_empty() {
            resolve_linux_variant(&id, ROLLING_DISTROS)
        } else {
            code
        }
    } else {
        resolve_linux_variant(&id, ROLLING_DISTROS)
    };

    Ok(format!("{id}_{variant}"))
}

#[cfg(target_os = "linux")]
fn resolve_linux_variant(id: &str, rolling: &[&str]) -> String {
    if rolling.iter().any(|d| *d == id) {
        "rolling".to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(target_os = "macos")]
fn detect_macos_os_suffix() -> Result<String> {
    use std::process::Command;
    let output = Command::new("sw_vers")
        .output()
        .context("failed to run sw_vers")?;
    if !output.status.success() {
        bail!("sw_vers returned non-zero exit status");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut product_name: Option<String> = None;
    let mut product_version: Option<String> = None;

    for line in stdout.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("ProductName") {
                product_name = Some(val.to_ascii_lowercase().replace(' ', ""));
            } else if key.eq_ignore_ascii_case("ProductVersion") {
                product_version = Some(val.to_ascii_lowercase());
            }
        }
    }

    let id = product_name.unwrap_or_else(|| "macos".into());
    let variant = product_version.ok_or_else(|| anyhow!("ProductVersion not found"))?;
    Ok(format!("{id}_{variant}"))
}

// --------------- Helpers ---------------

fn archive_if_exists(
    root_in_archive: &Path,
    file_path: &Path,
    tarball: &mut Builder<&mut GzEncoder<File>>,
) -> Result<()> {
    if !file_path.exists() {
        debug!(
            "Skipping {} (does not exist) while packing plugin",
            file_path.display()
        );
        return Ok(());
    }

    let archived_name = root_in_archive.join(
        file_path
            .file_name()
            .ok_or_else(|| anyhow!("Path without file name: {}", file_path.display()))?,
    );

    if file_path.is_dir() {
        tarball
            .append_dir_all(&archived_name, file_path)
            .with_context(|| format!("failed to append directory {}", file_path.display()))?;
    } else {
        let mut opened_file = File::open(file_path)
            .with_context(|| format!("failed to open file {}", file_path.display()))?;
        tarball
            .append_file(&archived_name, &mut opened_file)
            .with_context(|| format!("failed to append file {}", file_path.display()))?;
    }

    Ok(())
}

fn get_latest_plugin_version(plugin_dir: &Path) -> Result<String> {
    let cargo_toml_path = plugin_dir.join("Cargo.toml");
    let cargo_toml = fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("Failed to read {}", cargo_toml_path.display()))?;

    let parsed: Value = toml::from_str(&cargo_toml).context("Failed to parse Cargo.toml")?;

    let version = parsed
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            anyhow!(
                "Couldn't resolve version in Cargo.toml at {}",
                cargo_toml_path.display()
            )
        })?;

    Ok(version.to_string())
}
