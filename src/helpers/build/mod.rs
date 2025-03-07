use derive_builder::Builder;
use fs_extra::dir;
use fs_extra::dir::CopyOptions;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_TEMPLATE_NAME: &str = "manifest.yaml.template";

#[cfg(target_os = "linux")]
const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
const LIB_EXT: &str = "dylib";

// Get output path from env variable to get path up until debug/ or release/ folder
fn get_output_path() -> PathBuf {
    Path::new(&env::var("OUT_DIR").unwrap())
        .ancestors()
        .take(4)
        .collect()
}

#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default)]
    #[builder(setter(custom))]
    custom_assets: Vec<PathBuf>,
}

impl ParamsBuilder {
    pub fn custom_assets<I, S>(&mut self, assets: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let custom_assets: Vec<std::path::PathBuf> = assets
            .into_iter()
            .map(|asset| asset.as_ref().into())
            .collect();

        self.custom_assets = Some(custom_assets);

        self
    }
}

pub fn main(params: &Params) {
    let out_dir = get_output_path();
    let pkg_version = env::var("CARGO_PKG_VERSION").unwrap();
    let pkg_name = env::var("CARGO_PKG_NAME").unwrap();
    let plugin_path = out_dir.join(&pkg_name).join(&pkg_version);
    let out_manifest_path = plugin_path.join("manifest.yaml");
    let lib_name = format!("lib{}.{LIB_EXT}", pkg_name.replace('-', "_"));

    dir::remove(&plugin_path).unwrap();
    fs::create_dir_all(&plugin_path).unwrap();

    // Iterate through plugins version to find the latest
    // then replace symlinks with actual files
    for plugin_version_entry in fs::read_dir(out_dir.join(&pkg_name)).unwrap() {
        let plugin_version_path = plugin_version_entry.unwrap().path();
        if !plugin_version_path.is_dir() {
            continue;
        }

        for plugin_artefact in fs::read_dir(&plugin_version_path).unwrap() {
            let entry = plugin_artefact.unwrap();
            if !entry.file_type().unwrap().is_symlink()
                || (entry.file_name().to_str().unwrap() != lib_name)
            {
                continue;
            }

            // Need to remove symlink before copying in order to properly replace symlink with file
            let plugin_lib_path = plugin_version_path.join(&lib_name);
            let _ = fs::remove_file(&plugin_lib_path);
            fs::copy(out_dir.join(&lib_name), &plugin_lib_path).unwrap();

            break;
        }
    }

    // Generate folder with custom assets
    fs::create_dir(plugin_path.join("assets")).unwrap();

    // Generate new manifest.yaml and migrations from template
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let crate_dir = Path::new(&crate_dir);

    let migrations_dir = crate_dir.join("migrations");
    let mut migrations: Vec<String> = fs::read_dir(&migrations_dir)
        .map(|dir| {
            dir.map(|p| {
                p.unwrap()
                    .path()
                    .strip_prefix(crate_dir)
                    .unwrap()
                    .to_string_lossy()
                    .into()
            })
            .collect()
        })
        .unwrap_or_default();

    migrations.sort();

    // Copy migrations directory and manifest into newest plugin version
    if !migrations.is_empty() {
        let mut cp_opts = CopyOptions::new();
        cp_opts.overwrite = true;
        dir::copy(&migrations_dir, &plugin_path, &cp_opts).unwrap();
    }

    if crate_dir.join(MANIFEST_TEMPLATE_NAME).exists() {
        let template_path = crate_dir.join(MANIFEST_TEMPLATE_NAME);
        let template =
            fs::read_to_string(template_path).expect("template for manifest plugin not found");
        let template = liquid::ParserBuilder::with_stdlib()
            .build()
            .unwrap()
            .parse(&template)
            .expect("invalid manifest template");

        let template_ctx = liquid::object!({
            "version": pkg_version,
            "migrations": migrations,
        });

        fs::write(&out_manifest_path, template.render(&template_ctx).unwrap()).unwrap();
    } else {
        log::warn!(
            "Couldn't find manifest.yaml template at {}, skipping its generation...",
            crate_dir.display()
        );
    }

    // Create symlinks for newest plugin version, which would be created after build.rs script
    std::os::unix::fs::symlink(out_dir.join(&lib_name), plugin_path.join(lib_name)).unwrap();

    // Move custom assets into plugin folder
    for asset_path in &params.custom_assets {
        if !asset_path.exists() {
            println!(
                "cargo::warning=Couldn't find custom asset {} - skipping",
                asset_path.display(),
            );

            continue;
        }
        if asset_path.is_dir() {
            dir::copy(asset_path, plugin_path.join("assets"), &CopyOptions::new()).unwrap();
        } else {
            fs::copy(
                asset_path,
                plugin_path
                    .join("assets")
                    .join(asset_path.file_name().unwrap()),
            )
            .unwrap();
        }
    }

    // Trigger on Cargo.toml change in order not to run cargo update each time
    // version is changed
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed={MANIFEST_TEMPLATE_NAME}");
}
