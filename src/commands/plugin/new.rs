use anyhow::{bail, Context, Result};
use fs_extra::{dir, file};
use minijinja::Value;
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
static WS_CARGO_MANIFEST_TEMPLATE: &str = r#"[workspace]
resolver = "3"
members = [
    "{{ project_name }}",
]
"#;

static WS_PATHS_TO_MOVE: [&str; 6] = [
    "topology.toml",
    "picodata.yaml",
    ".gitignore",
    "rust-toolchain.toml",
    ".cargo/",
    "tmp/",
];

fn place_file(
    target_path: &Path,
    t_ctx: &minijinja::Value,
    entries: &[DirEntry<'_>],
) -> Result<()> {
    for entry in entries {
        match entry {
            DirEntry::Dir(inner_dir) => place_file(target_path, t_ctx, inner_dir.entries())?,
            DirEntry::File(inner_file) => {
                let mut env = minijinja::Environment::new();
                env.add_template(
                    "name",
                    inner_file
                        .contents_utf8()
                        .context("couldn't extract file contents")?,
                )?;
                let template = env.get_template("name")?;

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
                        .render(t_ctx)
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

fn workspace_init(root_path: &Path, project_name: &str, t_ctx: &Value) -> Result<()> {
    let cargo_toml_path = root_path.join("Cargo.toml");

    let mut cargo_toml =
        File::create(cargo_toml_path).context("failed to create Cargo.toml for workspace")?;

    let mut ws_env = minijinja::Environment::new();
    ws_env
        .add_template("cargo_manifect", WS_CARGO_MANIFEST_TEMPLATE)
        .expect("Can't parse cargo manifest template");
    let ws_template = ws_env
        .get_template("cargo_manifect")
        .expect("We just registered template for cargo manifest");

    cargo_toml.write_all(ws_template.render(t_ctx).unwrap().as_bytes())?;

    let subcrate_path = root_path.join(project_name);

    let move_file = |path: &Path| -> Result<u64> {
        let opts = file::CopyOptions {
            overwrite: true,
            ..Default::default()
        };
        let target = root_path.join(path.file_name().unwrap());
        file::move_file(path, target, &opts).context(format!(
            "failed to move {} to workspace dir",
            path.display()
        ))
    };

    let move_dir = |path: &Path| -> Result<u64> {
        let opts = dir::CopyOptions {
            overwrite: true,
            ..Default::default()
        };
        dir::move_dir(path, root_path, &opts).context(format!(
            "failed to move {} to workspace dir",
            path.display()
        ))
    };

    for path in WS_PATHS_TO_MOVE {
        let path = subcrate_path.join(path);
        if path.is_dir() {
            move_dir(&path)?;
        } else if path.is_file() {
            move_file(&path)?;
        } else {
            panic!("unsupported file type for moving")
        }
    }

    Ok(())
}

pub fn cmd(path: Option<&Path>, without_git: bool, init_workspace: bool) -> Result<()> {
    let path = match path {
        Some(p) => {
            if p.exists() {
                bail!("path {} already exists", p.to_string_lossy())
            }
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

    let templates_ctx = minijinja::context! {
        project_name => project_name,
    };

    place_file(&plugin_path, &templates_ctx, PLUGIN_TEMPLATE.entries())
        .context("failed to place the template")?;

    // init git in plugin repository
    if !without_git {
        let project_path = path.to_str().context("failed to extract project path")?;
        git(["-C", project_path, "init"])?;
        git(["-C", project_path, "add", "."])?;
    }

    if init_workspace {
        workspace_init(&path, project_name, &templates_ctx)
            .context("failed to initiate workspace")?;
    }

    Ok(())
}
