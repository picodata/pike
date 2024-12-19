use anyhow::{bail, Context, Result};
use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::Write,
    path::Path,
    process::Command,
};

use include_dir::{include_dir, Dir, DirEntry};

static PLUGIN_TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/plugin_template");

fn place_file(target_path: &Path, t_ctx: &liquid::Object, entries: &[DirEntry<'_>]) -> Result<()> {
    for entry in entries {
        match entry {
            DirEntry::Dir(inner_dir) => place_file(target_path, t_ctx, inner_dir.entries())?,
            DirEntry::File(inner_file) => {
                let template = liquid::ParserBuilder::with_stdlib()
                    .build()
                    .context("couldn't build from template")?
                    .parse(
                        inner_file
                            .contents_utf8()
                            .context("couldn't extract file contents")?,
                    )
                    .context(format!(
                        "invalid template {}",
                        inner_file.path().to_string_lossy()
                    ))?;

                // crutch for prevent excluding plugin_template directory from package
                // https://github.com/rust-lang/cargo/issues/8597
                let inner_file_path = if inner_file.path().ends_with("_Cargo.toml") {
                    &inner_file.path().parent().unwrap().join("Cargo.toml")
                } else {
                    inner_file.path()
                };

                let dest_path = Path::new(&target_path).join(inner_file_path);
                if let Some(dest_dir) = dest_path.parent() {
                    if !dest_dir.exists() {
                        std::fs::create_dir_all(dest_dir)?;
                    }
                }
                fs::write(
                    &dest_path,
                    template
                        .render(&t_ctx)
                        .context("failed to render the file")?,
                )
                .context(format!("couldn't write to {}", dest_path.display()))?;
            }
        }
    }

    Ok(())
}

fn git<I, S>(args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .args(args)
        .output()
        .context("failed to run git command, install git first")?;

    Ok(())
}

fn workspace_init(root_path: &Path, project_name: &str) -> Result<()> {
    let cargo_toml_path = root_path.join("Cargo.toml");

    let mut cargo_toml =
        File::create(cargo_toml_path).context("failed to create Cargo.toml for workspace")?;

    cargo_toml
        .write_all(format!("[workspace]\nmembers = [\n  \"{project_name}\",\n]").as_bytes())?;

    fs::copy(
        root_path.join(project_name).join("topology.toml"),
        root_path.join("topology.toml"),
    )
    .context("failed to move topology.toml to workspace dir")?;

    fs::copy(
        root_path.join(project_name).join("plugin_config.yaml"),
        root_path.join("plugin_config.yaml"),
    )
    .context("failed to move config.yaml to workspace dir")?;

    Ok(())
}

pub fn cmd(path: Option<&Path>, without_git: bool, init_workspace: bool) -> Result<()> {
    let path = match path {
        Some(p) => {
            if p.exists() {
                bail!("path {} already exists", p.to_string_lossy())
            };
            p.to_path_buf()
        }
        None => env::current_dir()?,
    };
    let project_name = &path
        .file_name()
        .context("failed to extract project name")?
        .to_str()
        .context("failed to parse filename to string")?;

    let plugin_path = if init_workspace {
        path.join(project_name)
    } else {
        path.clone()
    };

    std::fs::create_dir_all(&plugin_path)
        .context(format!("failed to create {}", plugin_path.display()))?;

    let templates_ctx = liquid::object!({
        "project_name": project_name,
    });

    place_file(&plugin_path, &templates_ctx, PLUGIN_TEMPLATE.entries())
        .context("failed to place the template")?;

    // init git in plugin repository
    if !without_git {
        let project_path = path.to_str().context("failed to extract project path")?;
        git(["-C", project_path, "init"])?;
        git(["-C", project_path, "add", "."])?;
    }

    if init_workspace {
        workspace_init(&path, project_name).context("failed to initiate workspace")?;
    }

    Ok(())
}
