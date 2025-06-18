use anyhow::{bail, Context, Result};
use flate2::bufread::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use lib::{cargo_build, BuildType};
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::{env, fs};
use tar::{Archive, Builder};
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

pub fn cmd(pack_debug: bool, target_dir: &PathBuf, pluging_path: &PathBuf) -> Result<()> {
    let root_dir = env::current_dir()?.join(pluging_path);

    let build_dir = if pack_debug {
        cargo_build(BuildType::Debug, target_dir, pluging_path)
            .context("building release version of plugin")?;
        Path::new(&root_dir).join(target_dir).join("debug")
    } else {
        cargo_build(BuildType::Release, target_dir, pluging_path)
            .context("building debug version of plugin")?;
        Path::new(&root_dir).join(target_dir).join("release")
    };

    let plugin_dir = root_dir.clone();

    let cargo_toml_path = root_dir.join("Cargo.toml");
    let cargo_toml_content = fs::read_to_string(&cargo_toml_path).context(format!(
        "Failed to read Cargo.toml in {}",
        &cargo_toml_path.display()
    ))?;

    let parsed_toml: Value = cargo_toml_content
        .parse()
        .context("Failed to parse Cargo.toml")?;

    if let Some(workspace) = parsed_toml.get("workspace") {
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

                    create_plugin_archive(&build_dir, &root_dir.join(member_str.unwrap()))?;
                }
            }
        }

        return Ok(());
    }

    create_plugin_archive(&build_dir, &plugin_dir)
}

/// Checks if provided path contains valid packed plugin archive
pub(crate) fn is_plugin_archive(test_path: &Path) -> Result<()> {
    if !test_path.is_file() {
        bail!("plugin archive path must be a file");
    }
    let file = File::options()
        .read(true)
        .write(false)
        .create(false)
        .open(test_path)
        .context("unable to open plugin archive candidate")?;
    let buf_reader = BufReader::new(file);
    let file_untar = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(file_untar);
    let Ok(archive_entries) = archive.entries() else {
        bail!("unable to read plugin archive candidate");
    };
    let mut has_manifest = false;
    let mut has_lib = false;
    let lib_suffix = format!(".{LIB_EXT}");
    for entry in archive_entries.filter_map(Result::ok) {
        if let Ok(entry_path) = entry.path() {
            // plugin_name / plugin_version / root_file_name
            if entry_path.components().count() == 3 {
                if let Some(last_part) = entry_path.components().last() {
                    has_manifest = has_manifest || last_part.as_os_str() == "manifest.yaml";
                    has_lib = has_lib
                        || last_part
                            .as_os_str()
                            .to_string_lossy()
                            .ends_with(&lib_suffix);
                }
            }
        }
        if has_manifest && has_lib {
            return Ok(());
        }
    }
    if !has_manifest {
        bail!("plugin archive candidate missing manifest");
    }
    if !has_lib {
        bail!("plugin archive candidate missing plugin library");
    }
    bail!("plugin archive candidate has invalid structure");
}

/// Validates and unpacks plugin(s) from shipping archive into destination path,
/// preserving archive structure. Does not creates destination path itself.
pub(crate) fn unpack_shipping_archive(src_path: &Path, dst_path: &Path) -> Result<()> {
    is_plugin_archive(src_path).with_context(|| {
        let (from, to) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("can not unpack shipping archive at {from} to {to}")
    })?;

    let file = File::options()
        .read(true)
        .write(false)
        .create(false)
        .open(src_path)
        .context("unable to open plugin archive")?;
    let buf_reader = BufReader::new(file);
    let decompressor = GzDecoder::new(buf_reader);

    // by default - override existing, preserve mtime
    let mut archive = Archive::new(decompressor);
    archive.unpack(dst_path).with_context(|| {
        let (from, to) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("failed to unpack shipping archive at {from} to {to}")
    })?;
    Ok(())
}

fn create_plugin_archive(build_dir: &Path, plugin_dir: &Path) -> Result<()> {
    let plugin_version = get_latest_plugin_version(plugin_dir)?;

    let cargo_manifest: CargoManifest = toml::from_str(
        &fs::read_to_string(plugin_dir.join("Cargo.toml")).context("failed to read Cargo.toml")?,
    )
    .context("failed to parse Cargo.toml")?;

    let package_name = cargo_manifest.package.name;
    let normalized_package_name = package_name.replace('-', "_");

    let plugin_build_dir = build_dir.join(&package_name).join(&plugin_version);

    let root_in_zip = Path::new(&package_name).join(plugin_version);

    let compressed_file_path = format!(
        "{}/{package_name}-{}.tar.gz",
        build_dir.display(),
        cargo_manifest.package.version
    );
    let compressed_file =
        File::create(compressed_file_path).context("failed to pack the plugin")?;

    let mut encoder = GzEncoder::new(compressed_file, Compression::best());

    let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");

    {
        let mut tarball = Builder::new(&mut encoder);

        archive_if_exists(
            &root_in_zip,
            &plugin_build_dir.join(&lib_name),
            &mut tarball,
        )?;
        archive_if_exists(
            &root_in_zip,
            &plugin_build_dir.join("manifest.yaml"),
            &mut tarball,
        )?;
        archive_if_exists(
            &root_in_zip,
            &plugin_build_dir.join("migrations"),
            &mut tarball,
        )?;

        let assets_path = &plugin_build_dir.join("assets");
        // no need to notify user if there is no assets folder
        if assets_path.exists() {
            for entry in fs::read_dir(assets_path)? {
                let entry = entry?;
                let entry_name = entry.file_name();
                archive_if_exists(&root_in_zip, &assets_path.join(entry_name), &mut tarball)?;
            }
        }
    }

    encoder.finish()?;

    Ok(())
}

fn archive_if_exists(
    root_in_zip: &Path,
    file_path: &Path,
    tarball: &mut Builder<&mut GzEncoder<File>>,
) -> Result<()> {
    if !file_path.exists() {
        log::info!(
            "Couldn't find {} while packing plugin - skipping.",
            file_path.display()
        );

        return Ok(());
    }

    let archived_file_name = root_in_zip.join(file_path.file_name().unwrap());

    if file_path.is_dir() {
        tarball
            .append_dir_all(archived_file_name, file_path)
            .context(format!(
                "failed to append directory: {} to archive",
                file_path.display()
            ))?;
    } else {
        let mut opened_file = File::open(file_path)
            .context(format!("failed to open file {}", &file_path.display()))?;

        tarball
            .append_file(archived_file_name, &mut opened_file)
            .context(format!(
                "failed to append file: {} to archive",
                file_path.display()
            ))?;
    }

    Ok(())
}

fn get_latest_plugin_version(plugin_dir: &Path) -> Result<String> {
    let cargo_toml =
        fs::read_to_string(plugin_dir.join("Cargo.toml")).expect("Failed to read Cargo.toml");

    let parsed: toml::Value = toml::de::from_str(&cargo_toml).expect("Failed to parse TOML");

    if let Some(package) = parsed.get("package") {
        if let Some(version) = package.get("version") {
            return Ok(version
                .to_string()
                .strip_prefix("\"")
                .unwrap()
                .strip_suffix("\"")
                .unwrap()
                .to_string());
        }
        bail!("Couldn't find version in plugin Cargo.toml");
    }

    bail!(
        "Couldn't resolve plugin version from Cargo.toml at {}",
        plugin_dir.display()
    )
}
