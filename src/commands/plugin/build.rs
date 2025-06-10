use std::fs::{self, FileType};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use lib::{cargo_build, BuildType};

use crate::commands::lib;

pub(crate) fn is_plugin_shipping_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!("path is not a plugin shipping directory");
    }
    let versioned_readers = fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|version| version.file_type().as_ref().is_ok_and(FileType::is_dir))
        .map(|path| fs::read_dir(path.path()));
    for version in versioned_readers.flatten() {
        let root_files = version.filter_map(Result::ok).collect::<Vec<_>>();
        let has_manifest = root_files.iter().any(|path| {
            path.file_name() == "manifest.yaml"
                && path.file_type().as_ref().is_ok_and(FileType::is_file)
        });
        if has_manifest {
            return Ok(());
        }
    }
    bail!("path does not match plugin dir structure")
}

fn copy_directory_tree_inner(
    src_path: &Path,
    dst_path: &Path,
    recursion: usize,
    rec_limit: usize,
) -> Result<()> {
    if recursion >= rec_limit {
        bail!("reached recursion limit of {rec_limit}");
    }
    let entries = fs::create_dir_all(dst_path)
        .and_then(|()| fs::read_dir(src_path))
        .with_context(|| {
            let src_path_display = src_path.to_string_lossy();
            format!("failed to read directory at {src_path_display}")
        })?;
    for entry in entries {
        let entry = entry.context("failed to read directory entry")?;
        let entry_path = entry.path();
        let entry_type = entry.file_type().context("failed to get file type")?;
        let dst_path = dst_path.join(entry.file_name());
        if entry_type.is_dir() {
            copy_directory_tree_inner(&entry_path, &dst_path, recursion + 1, rec_limit)?;
        } else {
            fs::copy(&entry_path, &dst_path).with_context(|| {
                let (from, to) = (entry_path.to_string_lossy(), dst_path.to_string_lossy());
                format!("failed to copy file {from} to {to} in directory tree")
            })?;
        }
    }
    Ok(())
}

pub(crate) fn copy_directory_tree(src_path: &Path, dst_path: &Path) -> Result<()> {
    copy_directory_tree_inner(src_path, dst_path, 0, 64).with_context(|| {
        let (src_path, dst_path) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("failed to copy directory tree from {src_path} to {dst_path}")
    })
}

pub fn cmd(release: bool, target_dir: &PathBuf, plugin_path: &PathBuf) -> Result<()> {
    let build_type = if release {
        BuildType::Release
    } else {
        BuildType::Debug
    };
    cargo_build(build_type, target_dir, plugin_path).context("building of plugin")
}
