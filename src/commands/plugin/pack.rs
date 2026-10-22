use crate::commands::lib::{cargo_build, BuildType, LIB_EXT};

use crate::helpers::cargo_manifest::{
    read_package_info, read_toml_document, MANIFEST_TEMPLATE_NAME,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{debug, info, warn};
use serde::Serialize;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{env, fs};
use tar::{Builder, Header};
use toml_edit::DocumentMut;

const BUILD_INFO_FILE_NAME: &str = "build_info.yaml";

/// Git state of the plugin repository at packing time, stored in the archive as `build_info.yaml`.
#[derive(Debug, PartialEq, Serialize)]
struct BuildInfo {
    commit: String,
    dirty: bool,
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

    let build_info = collect_build_info(&root_dir)
        .inspect_err(|reason| {
            warn!("Git commit info is unavailable, {BUILD_INFO_FILE_NAME} will not be added to the archive: {reason:#}");
        })
        .ok();
    if let Some(build_info) = &build_info {
        if build_info.dirty {
            warn!(
                "Repository working tree has uncommitted changes: commit {} in {BUILD_INFO_FILE_NAME} does not fully describe the archive contents",
                build_info.commit
            );
        }
        if no_build {
            info!("--no-build: {BUILD_INFO_FILE_NAME} reflects the repository state at packing time, not at build time");
        }
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
    let parsed_toml: DocumentMut = read_toml_document(&cargo_toml_path)?;

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
                if member_path.join(MANIFEST_TEMPLATE_NAME).exists() {
                    info!("Packing workspace member plugin: {}", member_path.display());
                    create_plugin_archive(
                        &build_root,
                        &member_path,
                        &root_dir,
                        None,
                        build_info.as_ref(),
                    )?;
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

    create_plugin_archive(
        &build_root,
        &root_dir,
        &root_dir,
        archive_name,
        build_info.as_ref(),
    )
}

fn create_plugin_archive(
    build_dir: &Path,
    plugin_dir: &Path,
    workspace_root: &Path,
    archive_name: Option<&PathBuf>,
    build_info: Option<&BuildInfo>,
) -> Result<()> {
    let package_info = read_package_info(plugin_dir, workspace_root)
        .context("failed to resolve package info for packaging")?;

    let package_name = package_info.name;
    let plugin_version = package_info.version;
    let normalized_package_name = package_name.replace('-', "_");
    let plugin_build_dir = build_dir.join(&package_name).join(&plugin_version);
    let root_in_archive = Path::new(&package_name).join(&plugin_version);

    validate_plugin_build_tree(&plugin_build_dir, &normalized_package_name)?;

    let compressed_file_path =
        resolve_archive_path(build_dir, archive_name, &package_name, &plugin_version)?;

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
        let manifest_path = plugin_build_dir.join("manifest.yaml");
        archive_if_exists(&root_in_archive, &manifest_path, &mut tarball)?;
        if let Some(build_info) = build_info {
            append_build_info(&root_in_archive, build_info, &manifest_path, &mut tarball)?;
        }
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

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn detect_os_suffix() -> Result<String> {
    detect_linux_freebsd_os_suffix()
}

#[cfg(target_os = "macos")]
fn detect_os_suffix() -> Result<String> {
    detect_macos_os_suffix()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "freebsd")))]
fn detect_os_suffix() -> Result<String> {
    bail!("unsupported operating system for packing plugin (supported: linux, macos)");
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn detect_linux_freebsd_os_suffix() -> Result<String> {
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

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn resolve_linux_variant(id: &str, rolling: &[&str]) -> String {
    if rolling.contains(&id) {
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

/// Runs `git -C <plugin_dir> <args>` and returns its raw output.
/// `Err` only when the process cannot be started.
fn run_git(plugin_dir: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(plugin_dir)
        .args(args)
        .output()
        .context("failed to run git, is it installed?")
}

/// Returns trimmed stdout of a finished `git <command>`, or `Err` with its stderr if it failed.
fn successful_git_stdout(command: &str, output: &Output) -> Result<String> {
    ensure!(
        output.status.success(),
        "git {command} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Collects commit hash and dirtiness of the git repository containing `plugin_dir`.
/// `Err` holds a human-readable reason why the info is unavailable.
fn collect_build_info(plugin_dir: &Path) -> Result<BuildInfo> {
    // `--verify --quiet` distinguishes the failures by exit code alone, independent of locale:
    // 1 - HEAD does not point to a commit (fresh `git init`), 128 - not a repository or other fatal error.
    // Plain `git rev-parse HEAD` prints "HEAD" to stdout even when it fails.
    let rev_parse = run_git(
        plugin_dir,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )?;
    ensure!(
        rev_parse.status.code() != Some(1),
        "repository has no commits (HEAD does not point to a commit)"
    );
    let commit = successful_git_stdout("rev-parse", &rev_parse)?;

    // `--untracked-files=normal` overrides a user's `status.showUntrackedFiles`,
    // so untracked files always make the tree dirty.
    // `--no-optional-locks` keeps `status` read-only: it does not take `index.lock` to refresh
    // the index, so it neither changes `.git/index` nor blocks git commands run concurrently.
    let status = run_git(
        plugin_dir,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ],
    )?;
    let dirty = !successful_git_stdout("status", &status)?.is_empty();

    Ok(BuildInfo { commit, dirty })
}

/// Serializes `build_info` and appends it as `build_info.yaml` to `root_in_archive` in the tarball.
/// The entry gets owner, mode and mtime of `metadata_source_path`, so it looks like
/// the other files of the archive, which `append_file` takes from the disk.
fn append_build_info(
    root_in_archive: &Path,
    build_info: &BuildInfo,
    metadata_source_path: &Path,
    tarball: &mut Builder<&mut GzEncoder<File>>,
) -> Result<()> {
    let content = serde_norway::to_string(build_info)
        .with_context(|| format!("failed to serialize {BUILD_INFO_FILE_NAME}"))?;
    let source_metadata = fs::metadata(metadata_source_path).with_context(|| {
        format!(
            "failed to read metadata of {}",
            metadata_source_path.display()
        )
    })?;

    let mut header = Header::new_gnu();
    header.set_metadata(&source_metadata);
    header.set_size(content.len() as u64);
    tarball
        .append_data(
            &mut header,
            root_in_archive.join(BUILD_INFO_FILE_NAME),
            content.as_bytes(),
        )
        .with_context(|| format!("failed to append {BUILD_INFO_FILE_NAME}"))
}

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

#[cfg(test)]
mod tests {
    use super::{
        append_build_info, archive_if_exists, collect_build_info, create_archive_path,
        generate_archive_path, resolve_archive_path, run_git, successful_git_stdout,
        validate_plugin_build_tree, BuildInfo, BUILD_INFO_FILE_NAME, LIB_EXT,
    };
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    fn run_git_successfully(dir: &Path, args: &[&str]) -> String {
        successful_git_stdout(&args.join(" "), &run_git(dir, args).unwrap()).unwrap()
    }

    fn init_git_repository(prefix: &str) -> PathBuf {
        let dir = tmp_dir(prefix);
        fs::create_dir_all(&dir).unwrap();
        run_git_successfully(&dir, &["init", "-q"]);
        dir
    }

    /// Commits every file of the repository without depending on the global git config.
    fn commit_all_files(dir: &Path) {
        run_git_successfully(dir, &["add", "-A"]);
        run_git_successfully(
            dir,
            &[
                "-c",
                "user.name=pike-tests",
                "-c",
                "user.email=pike-tests@localhost",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--no-verify",
                "-m",
                "init",
            ],
        );
    }

    fn head_commit(dir: &Path) -> String {
        run_git_successfully(dir, &["rev-parse", "HEAD"])
    }

    /// Creates a repository with one committed file `a.txt`.
    fn init_committed_repository(prefix: &str) -> PathBuf {
        let dir = init_git_repository(prefix);
        touch(&dir.join("a.txt"));
        commit_all_files(&dir);
        dir
    }

    fn append_line(path: &Path, line: &str) {
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        writeln!(file, "{line}").unwrap();
    }

    const SAMPLE_COMMIT: &str = "658364a999a9a051ccb1821fabf8af738b3364b9";

    #[test]
    fn build_info_serializes() {
        for dirty in [false, true] {
            let build_info = BuildInfo {
                commit: SAMPLE_COMMIT.into(),
                dirty,
            };
            assert_eq!(
                serde_norway::to_string(&build_info).unwrap(),
                format!("commit: {SAMPLE_COMMIT}\ndirty: {dirty}\n")
            );
        }
    }

    #[test]
    fn collect_build_info_fails_without_commits() {
        let dir = init_git_repository("bi-no-commits");
        touch(&dir.join("a.txt"));

        let reason = format!("{:#}", collect_build_info(&dir).unwrap_err());
        assert!(
            reason.contains("repository has no commits"),
            "Unexpected reason: {reason}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_build_info_clean_tree() {
        let dir = init_committed_repository("bi-clean");

        let build_info = collect_build_info(&dir).unwrap();
        assert_eq!(
            build_info,
            BuildInfo {
                commit: head_commit(&dir),
                dirty: false
            }
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_build_info_dirty_tree() {
        type MakeDirty = fn(&Path);
        let cases: [(&str, MakeDirty); 4] = [
            ("modified-tracked", |dir| {
                append_line(&dir.join("a.txt"), "changed");
            }),
            ("staged", |dir| {
                touch(&dir.join("b.txt"));
                run_git_successfully(dir, &["add", "b.txt"]);
            }),
            ("untracked", |dir| touch(&dir.join("b.txt"))),
            // `--untracked-files=normal` must override the user's config.
            ("show-untracked-files-no", |dir| {
                run_git_successfully(dir, &["config", "status.showUntrackedFiles", "no"]);
                touch(&dir.join("b.txt"));
            }),
        ];
        for (case, make_dirty) in cases {
            let dir = init_committed_repository(&format!("bi-dirty-{case}"));
            make_dirty(&dir);

            assert_eq!(
                collect_build_info(&dir).unwrap(),
                BuildInfo {
                    commit: head_commit(&dir),
                    dirty: true
                },
                "case {case}"
            );

            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn collect_build_info_does_not_write_index() {
        let dir = init_committed_repository("bi-no-index-write");
        // Same content with another mtime: `git status` refreshes the stat info of the entry
        // and, unless optional locks are disabled, writes the index back.
        fs::File::options()
            .write(true)
            .open(dir.join("a.txt"))
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000))
            .unwrap();
        let index_path = dir.join(".git").join("index");
        let index_before = fs::read(&index_path).unwrap();

        assert!(!collect_build_info(&dir).unwrap().dirty);
        assert_eq!(
            fs::read(&index_path).unwrap(),
            index_before,
            "git index must not be rewritten"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_build_info_ignores_gitignored_file() {
        let dir = init_git_repository("bi-gitignored");
        fs::write(dir.join(".gitignore"), "ignored.txt\n").unwrap();
        commit_all_files(&dir);
        touch(&dir.join("ignored.txt"));

        assert!(!collect_build_info(&dir).unwrap().dirty);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_build_info_in_subdirectory_sees_whole_repository() {
        let dir = init_git_repository("bi-subdirectory");
        let plugin_dir = dir.join("sub").join("plugin");
        touch(&plugin_dir.join("a.txt"));
        commit_all_files(&dir);

        assert_eq!(
            collect_build_info(&plugin_dir).unwrap(),
            BuildInfo {
                commit: head_commit(&dir),
                dirty: false
            }
        );

        touch(&dir.join("outside.txt"));
        assert!(collect_build_info(&plugin_dir).unwrap().dirty);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_build_info_copies_metadata_of_manifest_entry() {
        use flate2::{read::GzDecoder, write::GzEncoder, Compression};
        use std::io::Read;

        let dir = tmp_dir("bi-header");
        let plugin_build_dir = dir.join("build");
        let manifest_path = plugin_build_dir.join("manifest.yaml");
        touch(&manifest_path);
        // An mtime far from "now" tells the build time apart from the packing time.
        fs::File::options()
            .write(true)
            .open(&manifest_path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_600_000_000))
            .unwrap();

        let archive_path = dir.join("plugin.tar.gz");
        let root_in_archive = Path::new("plugin").join("0.1.0");
        let build_info = BuildInfo {
            commit: SAMPLE_COMMIT.into(),
            dirty: false,
        };
        {
            let mut encoder = GzEncoder::new(
                fs::File::create(&archive_path).unwrap(),
                Compression::default(),
            );
            let mut tarball = tar::Builder::new(&mut encoder);
            archive_if_exists(&root_in_archive, &manifest_path, &mut tarball).unwrap();
            append_build_info(&root_in_archive, &build_info, &manifest_path, &mut tarball).unwrap();
            tarball.finish().unwrap();
            drop(tarball);
            encoder.try_finish().unwrap();
        }

        let mut archive = tar::Archive::new(GzDecoder::new(fs::File::open(&archive_path).unwrap()));
        let mut manifest_header = None;
        let mut build_info_entry = None;
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let file_name = entry.path().unwrap().file_name().unwrap().to_owned();
            if file_name == "manifest.yaml" {
                manifest_header = Some(entry.header().clone());
            } else if file_name == BUILD_INFO_FILE_NAME {
                let mut content = String::new();
                entry.read_to_string(&mut content).unwrap();
                build_info_entry = Some((entry.header().clone(), content));
            }
        }
        let manifest_header = manifest_header.unwrap();
        let (build_info_header, content) = build_info_entry.unwrap();

        assert_eq!(content, serde_norway::to_string(&build_info).unwrap());
        let uid_gid_mode_mtime = |header: &tar::Header| {
            (
                header.uid().unwrap(),
                header.gid().unwrap(),
                header.mode().unwrap(),
                header.mtime().unwrap(),
            )
        };
        assert_eq!(
            uid_gid_mode_mtime(&build_info_header),
            uid_gid_mode_mtime(&manifest_header)
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
