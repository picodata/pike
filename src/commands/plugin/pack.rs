use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;
use std::{env, fs};
use tar::Builder;

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

fn cargo_build(build_args: Vec<&str>) -> Result<()> {
    let output = Command::new("cargo")
        .args(build_args)
        .output()
        .context("running cargo build")?;
    if !output.status.success() {
        bail!("build error: {}", String::from_utf8_lossy(&output.stderr));
    }

    Ok(())
}

pub fn cmd(pack_release: bool) -> Result<()> {
    let root_dir = env::current_dir()?;
    let plugin_name = &root_dir
        .file_name()
        .context("extracting project name")?
        .to_str()
        .context("parsing filename to string")?;

    let build_dir = if pack_release {
        cargo_build(vec!["build", "--release"]).context("building release version of plugin")?;
        Path::new(&root_dir).join("target").join("release")
    } else {
        cargo_build(vec!["build"]).context("building debug version of plugin")?;
        Path::new(&root_dir).join("target").join("debug")
    };

    let mut manifest_dir = root_dir.clone();
    // Workaround for case, when plugins is a subcrate of workspace
    {
        let cargo_toml_file: File = File::open(root_dir.join("Cargo.toml")).unwrap();
        let toml_reader = BufReader::new(cargo_toml_file);

        for line in toml_reader.lines() {
            let line = line?;
            if line.contains("workspace") {
                manifest_dir = root_dir.join(plugin_name);
                break;
            }
        }
    }

    let cargo_manifest: CargoManifest = toml::from_str(
        &fs::read_to_string(manifest_dir.join("Cargo.toml"))
            .context("failed to read Cargo.toml")?,
    )
    .context("failed to parse Cargo.toml")?;

    let normalized_package_name = cargo_manifest.package.name.replace("-", "_");

    let compressed_file = File::create(format!(
        "target/{}-{}.tar.gz",
        &normalized_package_name, cargo_manifest.package.version
    ))
    .context("failed to pack the plugin")?;

    let mut encoder = GzEncoder::new(compressed_file, Compression::best());

    let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");
    let mut lib_file =
        File::open(build_dir.join(&lib_name)).context(format!("failed to open {}", lib_name))?;

    let mut manifest_file =
        File::open(build_dir.join("manifest.yaml")).context("failed to open file manifest.yaml")?;
    {
        let mut tarball = Builder::new(&mut encoder);

        tarball
            .append_file(lib_name, &mut lib_file)
            .context(format!(
                "failed to append lib{normalized_package_name}.{LIB_EXT}"
            ))?;

        tarball
            .append_file("manifest.yaml", &mut manifest_file)
            .context("failed to add manifest.yaml to archive")?;

        tarball
            .append_dir_all("migrations", build_dir.join("migrations"))
            .context("failed to append \"migrations\" to archive")?;
    }

    encoder.finish()?;

    Ok(())
}
