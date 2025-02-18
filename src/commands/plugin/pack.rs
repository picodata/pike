use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use lib::{cargo_build, BuildType};
use serde::Deserialize;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::{env, fs};
use tar::Builder;
use toml::Value;

use crate::commands::lib;

#[derive(Deserialize)]
struct PackageInfo {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct CargoManifest {
    package: PackageInfo,
}

#[cfg(target_os = "linux")]
const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
const LIB_EXT: &str = "dylib";

pub fn cmd(pack_debug: bool, target_dir: &PathBuf) -> Result<()> {
    let root_dir = env::current_dir()?;

    let build_dir = if pack_debug {
        cargo_build(BuildType::Debug, target_dir).context("building release version of plugin")?;
        Path::new(&root_dir).join(target_dir).join("debug")
    } else {
        cargo_build(BuildType::Release, target_dir).context("building debug version of plugin")?;
        Path::new(&root_dir).join(target_dir).join("release")
    };

    let plugin_dir = root_dir.clone();

    let cargo_toml_path = Path::new("Cargo.toml");
    let cargo_toml_content =
        fs::read_to_string(cargo_toml_path).expect("Failed to read Cargo.toml");

    let parsed_toml: Value = cargo_toml_content
        .parse()
        .context("Failed to parse Cargo.toml")?;

    if let Some(workspace) = parsed_toml.get("workspace") {
        if let Some(members) = workspace.get("members") {
            if let Some(members_array) = members.as_array() {
                for member in members_array {
                    if let Some(member_str) = member.as_str() {
                        create_plugin_archive(&build_dir, &root_dir.join(member_str))?;
                    }
                }
            }
        }
        return Ok(());
    }

    create_plugin_archive(&build_dir, &plugin_dir)
}

fn create_plugin_archive(build_dir: &Path, plugin_dir: &Path) -> Result<()> {
    let cargo_manifest: CargoManifest = toml::from_str(
        &fs::read_to_string(plugin_dir.join("Cargo.toml")).context("failed to read Cargo.toml")?,
    )
    .context("failed to parse Cargo.toml")?;

    let normalized_package_name = cargo_manifest.package.name.replace('-', "_");

    let compressed_file = File::create(format!(
        "{}/{}-{}.tar.gz",
        build_dir.display(),
        &normalized_package_name,
        cargo_manifest.package.version
    ))
    .context("failed to pack the plugin")?;

    let mut encoder = GzEncoder::new(compressed_file, Compression::best());

    let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");

    {
        let mut tarball = Builder::new(&mut encoder);

        if build_dir.join(&lib_name).exists() {
            let mut lib_file = File::open(build_dir.join(&lib_name))
                .context(format!("failed to open {lib_name}"))?;
            tarball
                .append_file(lib_name, &mut lib_file)
                .context(format!(
                    "failed to append lib{normalized_package_name}.{LIB_EXT}"
                ))?;
        }

        if build_dir.join("manifest.yaml").exists() {
            let mut manifest_file = File::open(build_dir.join("manifest.yaml"))
                .context("failed to open file manifest.yaml")?;
            tarball
                .append_file("manifest.yaml", &mut manifest_file)
                .context("failed to add manifest.yaml to archive")?;
        }

        if build_dir.join("migrations").exists() {
            tarball
                .append_dir_all("migrations", build_dir.join("migrations"))
                .context("failed to append \"migrations\" to archive")?;
        }
    }

    encoder.finish()?;

    Ok(())
}
