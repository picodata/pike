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
    custom_assets: Vec<(PathBuf, PathBuf)>,
}

impl ParamsBuilder {
    pub fn custom_assets<I, S>(&mut self, assets: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.custom_assets_with_targets(assets.into_iter().map(|asset| {
            (
                asset.as_ref().to_string(),
                Path::new(asset.as_ref())
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            )
        }))
    }

    pub fn custom_assets_with_targets<I, S>(&mut self, assets: I) -> &mut Self
    where
        I: IntoIterator<Item = (S, S)>,
        S: AsRef<str>,
    {
        let custom_assets_with_targets: Vec<(PathBuf, PathBuf)> = assets
            .into_iter()
            .map(|(from, to)| (from.as_ref().into(), to.as_ref().into()))
            .collect();

        let mut t = self.custom_assets.take().unwrap_or_default();
        t.extend(custom_assets_with_targets);
        self.custom_assets = Some(t);

        self
    }
}

fn add_custom_assets(custom_assets: &Vec<(PathBuf, PathBuf)>, plugin_path: &Path) {
    for (from_asset_path, to_asset_path) in custom_assets {
        if !from_asset_path.exists() {
            println!(
                "cargo::warning=Couldn't find custom asset {} - skipping",
                from_asset_path.display(),
            );

            continue;
        }

        // Check if `to_asset_path` is out of bounds of the `assets` folder
        if to_asset_path.components().any(|comp| {
            matches!(comp, std::path::Component::ParentDir)
                || matches!(comp, std::path::Component::RootDir)
        }) {
            println!(
                "cargo::warning=Path to a custom asset destination {} goes out of the assets folder - skipping",
                to_asset_path.display()
            );

            continue;
        }

        let destination = plugin_path.join("assets").join(to_asset_path);

        println!("cargo::rerun-if-changed={}", from_asset_path.display());

        if from_asset_path.is_dir() {
            if !destination.exists() {
                fs::create_dir_all(&destination).unwrap();
            }

            let mut options = fs_extra::dir::CopyOptions::new();
            options.overwrite = true;
            options.copy_inside = true;
            options.content_only = true;
            fs_extra::dir::copy(from_asset_path, &destination, &options).unwrap();
        } else {
            // Create a directory into which a file should be copied
            let parent_destination_directory = destination.parent().unwrap();
            if !parent_destination_directory.exists() {
                fs::create_dir_all(parent_destination_directory).unwrap();
            }
            fs::copy(from_asset_path, destination).unwrap();
        }
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
        println!("cargo::rerun-if-changed={}", migrations_dir.display());
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

    add_custom_assets(&params.custom_assets, &plugin_path);

    // Trigger on Cargo.toml change in order not to run cargo update each time
    // version is changed
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed={MANIFEST_TEMPLATE_NAME}");
}
