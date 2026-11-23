use super::compute_relative_path_from_base_to_target;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{Document, DocumentMut, InlineTable, Value};

/// Points the project's pike build dependency and those of its members to the local pike project.
/// `plugin_path` is absolute or relative to the test process's working directory.
///
/// # Panics
/// Panics if an expected Cargo.toml cannot be read, parsed, or updated.
pub fn rewrite_project_and_workspace_member_pike_build_dependencies_to_local_pike_project(
    plugin_path: &OsStr,
) {
    let pike_project_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .expect("failed to resolve the pike project root");
    let mut pending_project_directories = vec![PathBuf::from(plugin_path)];
    let mut prepared_cargo_paths = HashSet::new();

    while let Some(project_directory) = pending_project_directories.pop() {
        let cargo_path = project_directory.join("Cargo.toml");
        let canonical_cargo_path = cargo_path
            .canonicalize()
            .unwrap_or_else(|error| panic!("failed to resolve {}: {error}", cargo_path.display()));
        if prepared_cargo_paths.contains(&canonical_cargo_path) {
            continue;
        }
        let workspace_member_directories =
            rewrite_pike_build_dependency_if_changed_and_read_workspace_members(
                &canonical_cargo_path,
                &pike_project_root,
            )
            .unwrap_or_else(|error| panic!("failed to prepare local pike dependency: {error:#}"));
        pending_project_directories.extend(workspace_member_directories);
        prepared_cargo_paths.insert(canonical_cargo_path);
    }
}

fn read_pike_build_dependency_path(plugin_directory: &Path) -> PathBuf {
    let cargo_contents = fs::read_to_string(plugin_directory.join("Cargo.toml")).unwrap();
    let cargo_document = cargo_contents.parse::<DocumentMut>().unwrap();
    PathBuf::from(
        cargo_document["build-dependencies"]["picodata-pike"]["path"]
            .as_str()
            .expect("pike build dependency must have a local path"),
    )
}

pub fn assert_pike_build_dependency_resolves_to_local_pike_project(plugin_directory: &Path) {
    let pike_build_dependency_path = read_pike_build_dependency_path(plugin_directory);
    assert!(pike_build_dependency_path.is_relative());
    assert_eq!(
        plugin_directory
            .join(pike_build_dependency_path)
            .canonicalize()
            .unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap()
    );
}

fn rewrite_pike_build_dependency_if_changed_and_read_workspace_members(
    cargo_path: &Path,
    pike_project_root: &Path,
) -> Result<Vec<PathBuf>> {
    let mut cargo_contents = fs::read_to_string(cargo_path)
        .with_context(|| format!("failed to read {}", cargo_path.display()))?;
    let cargo_document = Document::parse(cargo_contents.as_str())
        .with_context(|| format!("failed to parse {}", cargo_path.display()))?;
    let project_directory = cargo_path
        .parent()
        .context("Cargo.toml must have a parent directory")?;
    let workspace_member_directories =
        read_workspace_member_directories(&cargo_document, cargo_path, project_directory)?;
    let Some(pike_build_dependency) = cargo_document
        .get("build-dependencies")
        .and_then(|dependencies| dependencies.get("picodata-pike"))
    else {
        return Ok(workspace_member_directories);
    };

    let relative_path_to_pike_project_root =
        compute_relative_path_from_base_to_target(project_directory, pike_project_root);
    let relative_path_to_pike_project_root = relative_path_to_pike_project_root
        .to_str()
        .context("relative path to the pike project root must be valid UTF-8")?;
    let (replacement_span, replacement_contents) = if pike_build_dependency.is_str() {
        let mut local_pike_build_dependency = InlineTable::new();
        local_pike_build_dependency.insert("path", Value::from(relative_path_to_pike_project_root));
        local_pike_build_dependency.fmt();
        (
            pike_build_dependency.span(),
            local_pike_build_dependency.to_string(),
        )
    } else {
        let (pike_build_dependency_path_value, pike_build_dependency_path) = pike_build_dependency
            .as_inline_table()
            .and_then(|dependency| dependency.get("path"))
            .and_then(|path_value| path_value.as_str().map(|path| (path_value, path)))
            .with_context(|| {
                format!(
                    "expected a version string or inline path for picodata-pike in {}",
                    cargo_path.display()
                )
            })?;
        if Path::new(pike_build_dependency_path).is_relative()
            && project_directory
                .join(pike_build_dependency_path)
                .canonicalize()
                .is_ok_and(|resolved_dependency_directory| {
                    resolved_dependency_directory == pike_project_root
                })
        {
            return Ok(workspace_member_directories);
        }
        (
            pike_build_dependency_path_value.span(),
            Value::from(relative_path_to_pike_project_root).to_string(),
        )
    };
    let replacement_span = replacement_span.context("dependency must have a source span")?;
    if cargo_contents[replacement_span.clone()] != replacement_contents {
        // Replace only the value so comments, CRLF, and final newlines remain untouched.
        cargo_contents.replace_range(replacement_span, &replacement_contents);
        fs::write(cargo_path, cargo_contents)
            .with_context(|| format!("failed to write {}", cargo_path.display()))?;
    }
    Ok(workspace_member_directories)
}

fn read_workspace_member_directories(
    cargo_document: &Document<&str>,
    cargo_path: &Path,
    workspace_directory: &Path,
) -> Result<Vec<PathBuf>> {
    let Some(members) = cargo_document
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
    else {
        return Ok(Vec::new());
    };
    let members = members.as_array().with_context(|| {
        format!(
            "workspace.members must be an array in {}",
            cargo_path.display()
        )
    })?;
    members
        .iter()
        .map(|member| {
            member
                .as_str()
                .map(|member_path| workspace_directory.join(member_path))
                .with_context(|| {
                    format!(
                        "workspace member must be a path string in {}",
                        cargo_path.display()
                    )
                })
        })
        .collect()
}
