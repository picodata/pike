use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;
use toml_edit::{DocumentMut, Item};

/// Resolved `[package]` name and version of a crate's manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
}

/// Reads `name`/`version` from `<manifest_dir>/Cargo.toml`.
///
/// Cargo allows a crate to inherit its version from the workspace via
/// `version.workspace = true`, in which case the field is a TOML table
/// (`{ workspace = true }`) rather than a string. When that happens, the
/// actual version is looked up in `<workspace_root>/Cargo.toml`'s
/// `[workspace.package].version`.
pub fn read_package_info(manifest_dir: &Path, workspace_root: &Path) -> Result<PackageInfo> {
    let manifest_path = manifest_dir.join("Cargo.toml");
    let doc = read_toml_document(&manifest_path)?;

    let package = doc
        .get("package")
        .and_then(Item::as_table_like)
        .ok_or_else(|| anyhow!("no [package] table in {}", manifest_path.display()))?;

    let name = package
        .get("name")
        .and_then(Item::as_str)
        .ok_or_else(|| anyhow!("no package.name in {}", manifest_path.display()))?
        .to_string();

    let version_item = package
        .get("version")
        .ok_or_else(|| anyhow!("no package.version in {}", manifest_path.display()))?;

    let version = if let Some(version) = version_item.as_str() {
        version.to_string()
    } else {
        let inherits_workspace = version_item
            .as_table_like()
            .and_then(|table| table.get("workspace"))
            .and_then(Item::as_bool)
            .unwrap_or(false);
        if !inherits_workspace {
            bail!(
                "unsupported package.version format in {}: expected a string or `{{ workspace = true }}`",
                manifest_path.display()
            );
        }
        resolve_workspace_version(workspace_root)?
    };

    Ok(PackageInfo { name, version })
}

fn resolve_workspace_version(workspace_root: &Path) -> Result<String> {
    let manifest_path = workspace_root.join("Cargo.toml");
    let doc = read_toml_document(&manifest_path)?;

    doc.get("workspace")
        .and_then(Item::as_table_like)
        .and_then(|workspace| workspace.get("package"))
        .and_then(Item::as_table_like)
        .and_then(|package| package.get("version"))
        .and_then(Item::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "package.version is inherited from the workspace, but no workspace.package.version was found in {}",
                manifest_path.display()
            )
        })
}

pub(crate) fn read_toml_document(path: &Path) -> Result<DocumentMut> {
    fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(prefix: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut dir = std::env::temp_dir();
        dir.push(format!("pike-cargo-manifest-ut-{prefix}-{ts}"));
        dir
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn reads_literal_version() {
        let dir = tmp_dir("literal");
        write(
            &dir.join("Cargo.toml"),
            r#"
            [package]
            name = "some-plugin"
            version = "1.2.3"
            "#,
        );

        let info = super::read_package_info(&dir, &dir).unwrap();
        assert_eq!(info.name, "some-plugin");
        assert_eq!(info.version, "1.2.3");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_workspace_inherited_version() {
        let workspace_root = tmp_dir("workspace-root");
        write(
            &workspace_root.join("Cargo.toml"),
            r#"
            [workspace]
            members = ["plugin"]

            [workspace.package]
            version = "1.0.5"
            "#,
        );

        let plugin_dir = workspace_root.join("plugin");
        write(
            &plugin_dir.join("Cargo.toml"),
            r#"
            [package]
            name = "plugin"
            version.workspace = true
            "#,
        );

        let info = super::read_package_info(&plugin_dir, &workspace_root).unwrap();
        assert_eq!(info.name, "plugin");
        assert_eq!(info.version, "1.0.5");

        let _ = fs::remove_dir_all(&workspace_root);
    }

    #[test]
    fn errors_when_workspace_version_is_missing() {
        let workspace_root = tmp_dir("workspace-missing-version");
        write(
            &workspace_root.join("Cargo.toml"),
            r#"
            [workspace]
            members = ["plugin"]
            "#,
        );

        let plugin_dir = workspace_root.join("plugin");
        write(
            &plugin_dir.join("Cargo.toml"),
            r#"
            [package]
            name = "plugin"
            version.workspace = true
            "#,
        );

        let err = super::read_package_info(&plugin_dir, &workspace_root).unwrap_err();
        assert!(
            format!("{err:#}").contains("workspace.package.version"),
            "unexpected error: {err:#}"
        );

        let _ = fs::remove_dir_all(&workspace_root);
    }
}
