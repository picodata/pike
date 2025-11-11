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

/// Validate that pre-built plugin shipping directory contains required files
/// Required: manifest.yaml and `lib{normalized_package_name}.{LIB_EXT}`
fn validate_plugin_build_tree(
    plugin_build_dir: &Path,
    normalized_package_name: &str,
) -> Result<()> {
    if !plugin_build_dir.exists() {
        bail!(
            "Build output directory not found: {}. Build the plugin first or remove --no-build.",
            plugin_build_dir.display()
        );
    }

    let lib_name = format!("lib{normalized_package_name}.{LIB_EXT}");
    let lib_path = plugin_build_dir.join(&lib_name);
    if !lib_path.exists() {
        bail!(
            "Missing plugin library '{}' in {}. Build the plugin first or remove --no-build.",
            lib_name,
            plugin_build_dir.display()
        );
    }

    let manifest_path = plugin_build_dir.join("manifest.yaml");
    if !manifest_path.exists() {
        bail!(
            "Missing manifest.yaml in {}. Build the plugin first or remove --no-build.",
            plugin_build_dir.display()
        );
    }

    Ok(())
}

pub fn cmd(
    pack_debug: bool,
    target_dir: &PathBuf,
    plugin_path: &PathBuf,
    no_build: bool,
    archive_name: Option<&PathBuf>,
) -> Result<()> {
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

    if no_build {
        info!("--no-build: skipping cargo build for plugin pack");
    } else {
        cargo_build(build_type, target_dir, plugin_path)
            .with_context(|| format!("building {build_type} version of plugin"))?;
    }

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
        if archive_name.is_some() {
            bail!(
                "--archive-name is not supported for workspaces (multiple archives are produced)"
            );
        }

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
                    create_plugin_archive(&build_root, &member_path, None)?;
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

    create_plugin_archive(&build_root, &root_dir, archive_name)
}

fn create_plugin_archive(
    build_dir: &Path,
    plugin_dir: &Path,
    archive_name: Option<&PathBuf>,
) -> Result<()> {
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

    validate_plugin_build_tree(&plugin_build_dir, &normalized_package_name)?;

    let compressed_file_path = resolve_archive_path(
        build_dir,
        archive_name,
        &package_name,
        &cargo_manifest.package.version,
    )?;

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

    let parent = compressed_file_path
        .parent()
        .ok_or_else(|| anyhow!("failed to resolve archive parent directory"))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to ensure archive parent directory {}",
            parent.display()
        )
    })?;

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

fn resolve_archive_path(
    build_dir: &Path,
    archive_name: Option<&PathBuf>,
    package_name: &str,
    package_version: &str,
) -> Result<PathBuf> {
    if let Some(name) = archive_name {
        // Create path with user-specified archive name.
        create_archive_path(build_dir, name)
    } else {
        // Generate path with OS suffix.
        generate_archive_path(build_dir, package_name, package_version)
    }
}

fn create_archive_path(build_dir: &Path, archive_name: &Path) -> Result<PathBuf> {
    let mut dest = if archive_name.is_absolute() {
        archive_name.to_path_buf()
    } else {
        build_dir.join(archive_name)
    };

    let name = dest
        .file_name()
        .ok_or_else(|| {
            anyhow!(
                "invalid archive name (no filename component): {}",
                dest.display()
            )
        })?
        .to_string_lossy()
        .to_string();
    if !name.ends_with(".tar.gz") {
        dest.set_file_name(format!("{name}.tar.gz"));
    }
    Ok(dest)
}

fn generate_archive_path(
    build_dir: &Path,
    package_name: &str,
    package_version: &str,
) -> Result<PathBuf> {
    // Default archive name with OS suffix.
    let os_suffix = detect_os_suffix().context("failed to detect OS for archive naming")?;
    let archive_filename = format!("{package_name}_{package_version}-{os_suffix}.tar.gz");
    Ok(build_dir.join(archive_filename))
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
        "cachyos",
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

#[cfg(test)]
mod tests {
    use super::{
        create_archive_path, generate_archive_path, resolve_archive_path,
        validate_plugin_build_tree, LIB_EXT,
    };
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(prefix: &str) -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = std::env::temp_dir();
        dir.push(format!("pike-pack-ut-{prefix}-{ts}"));
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(path).unwrap();
        let _ = f.write_all(b"");
    }

    fn make_build_tree(
        base: &Path,
        pkg: &str,
        ver: &str,
        with_manifest: bool,
        with_lib: bool,
    ) -> PathBuf {
        let build_dir = base.join(pkg).join(ver);
        fs::create_dir_all(&build_dir).unwrap();

        if with_manifest {
            touch(&build_dir.join("manifest.yaml"));
        }
        if with_lib {
            let libname = format!("lib{}.{LIB_EXT}", pkg.replace('-', "_"));
            touch(&build_dir.join(libname));
        }
        fs::create_dir_all(build_dir.join("migrations")).unwrap();

        build_dir
    }

    #[test]
    fn validate_ok_when_all_required_files_exist() {
        let base = tmp_dir("ok");
        let pkg = "some-plugin";
        let ver = "0.1.0";
        let plugin_build_dir = make_build_tree(&base, pkg, ver, true, true);

        let res = validate_plugin_build_tree(&plugin_build_dir, &pkg.replace('-', "_"));
        assert!(res.is_ok(), "Expected OK, got error: {res:?}");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn validate_fails_if_dir_missing() {
        let base = tmp_dir("missing-dir");
        let non_existing = base.join("nope/0.0.0");
        let res = validate_plugin_build_tree(&non_existing, "nope");
        assert!(res.is_err(), "Expected error for missing dir");
        let msg = format!("{res:?}");
        assert!(
            msg.contains("Build output directory not found"),
            "Unexpected error message: {msg}"
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn validate_fails_if_manifest_missing() {
        let base = tmp_dir("missing-manifest");
        let pkg = "x-plugin";
        let ver = "1.2.3";
        let plugin_build_dir = make_build_tree(&base, pkg, ver, false, true);

        let res = validate_plugin_build_tree(&plugin_build_dir, &pkg.replace('-', "_"));
        assert!(res.is_err(), "Expected error for missing manifest");
        let msg = format!("{res:?}");
        assert!(
            msg.contains("Missing manifest.yaml"),
            "Unexpected error message: {msg}"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn validate_fails_if_lib_missing() {
        let base = tmp_dir("missing-lib");
        let pkg = "another-plugin";
        let ver = "9.9.9";
        let plugin_build_dir = make_build_tree(&base, pkg, ver, true, false);

        let res = validate_plugin_build_tree(&plugin_build_dir, &pkg.replace('-', "_"));
        assert!(res.is_err(), "Expected error for missing lib");
        let msg = format!("{res:?}");
        let expected_lib = format!("lib{}.{LIB_EXT}", pkg.replace('-', "_"));
        assert!(
            msg.contains("Missing plugin library") && msg.contains(&expected_lib),
            "Unexpected error message: {msg}"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_archive_relative_with_ext_goes_into_build_dir() {
        let build_dir = PathBuf::from("/tmp/build/rel");
        let dest = resolve_archive_path(
            &build_dir,
            Some(&PathBuf::from("custom.tar.gz")),
            "pkg",
            "0.1.0",
        )
        .unwrap();
        assert_eq!(dest, build_dir.join("custom.tar.gz"));
    }

    #[test]
    fn resolve_archive_relative_without_ext_appends_tar_gz() {
        let build_dir = PathBuf::from("/tmp/build/rel");
        let dest = resolve_archive_path(&build_dir, Some(&PathBuf::from("custom")), "pkg", "0.1.0")
            .unwrap();
        assert_eq!(dest, build_dir.join("custom.tar.gz"));
    }

    #[test]
    fn resolve_archive_absolute_without_ext_appends_tar_gz() {
        let build_dir = PathBuf::from("/tmp/build/rel");
        let dest = resolve_archive_path(
            &build_dir,
            Some(&PathBuf::from("/var/tmp/out/custom-name")),
            "pkg",
            "0.1.0",
        )
        .unwrap();
        assert_eq!(dest, PathBuf::from("/var/tmp/out/custom-name.tar.gz"));
    }
    #[test]
    fn create_archive_path_keeps_absolute_path_with_ext() {
        let build_dir = PathBuf::from("/tmp/build/rel");
        let dest = create_archive_path(&build_dir, Path::new("/var/tmp/out/file.tar.gz")).unwrap();
        assert_eq!(dest, PathBuf::from("/var/tmp/out/file.tar.gz"));
    }

    #[test]
    fn generate_archive_path_includes_suffix() {
        let p = generate_archive_path(Path::new("/tmp/build/rel"), "pkg", "0.1.0").unwrap();
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("pkg_0.1.0-") && name.ends_with(".tar.gz"));
    }
}
