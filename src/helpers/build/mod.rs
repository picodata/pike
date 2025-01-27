use derive_builder::Builder;
use fs_extra::dir;
use fs_extra::dir::CopyOptions;
use std::env;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const MANIFEST_TEMPLATE_NAME: &str = "manifest.yaml.template";

#[cfg(target_os = "linux")]
const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
const LIB_EXT: &str = "dylib";

fn get_output_path() -> PathBuf {
    let manifest_dir_string = env::var("CARGO_MANIFEST_DIR").unwrap();
    let build_type = env::var("PROFILE").unwrap();

    // Workaround for case, when plugins is a subcrate of workspace
    if !Path::new("../Cargo.toml").exists() {
        return Path::new(&manifest_dir_string)
            .join("target")
            .join(build_type);
    }

    let cargo_toml_file: File = File::open("../Cargo.toml").unwrap();
    let toml_reader = BufReader::new(cargo_toml_file);
    for line in toml_reader.lines() {
        let line = line.unwrap();
        if line.contains("workspace") {
            return Path::new(&manifest_dir_string)
                .join("..")
                .join("target")
                .join(build_type);
        }
    }

    Path::new(&manifest_dir_string)
        .join("target")
        .join(build_type)
}

#[derive(Debug, Builder)]
pub struct Params {}

pub fn main(_params: &Params) {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let crate_dir = Path::new(&crate_dir);

    let template_path = crate_dir.join(MANIFEST_TEMPLATE_NAME);
    let template =
        fs::read_to_string(template_path).expect("template for manifest plugin not found");
    let template = liquid::ParserBuilder::with_stdlib()
        .build()
        .unwrap()
        .parse(&template)
        .expect("invalid manifest template");

    let migrations_dir = crate_dir.join("migrations");
    let migrations: Vec<String> = match fs::read_dir(&migrations_dir) {
        Ok(dir) => dir
            .map(|path| {
                path.unwrap()
                    .path()
                    .strip_prefix(crate_dir)
                    .unwrap()
                    .to_string_lossy()
                    .into()
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    let pkg_version = env::var("CARGO_PKG_VERSION").unwrap();

    let template_ctx = liquid::object!({
        "version": pkg_version,
        "migrations": migrations,
    });

    let out_dir = get_output_path();
    let out_manifest_path = Path::new(&out_dir).join("manifest.yaml");
    fs::write(&out_manifest_path, template.render(&template_ctx).unwrap()).unwrap();

    if !migrations.is_empty() {
        let mut cp_opts = CopyOptions::new();
        cp_opts.overwrite = true;
        dir::copy(migrations_dir, &out_dir, &cp_opts).unwrap();
    }

    // create symbolic link
    let pkg_name = env::var("CARGO_PKG_NAME").unwrap();
    let plugin_path = out_dir.join(&pkg_name).join(pkg_version);
    dir::remove(&plugin_path).unwrap();
    fs::create_dir_all(&plugin_path).unwrap();
    std::os::unix::fs::symlink(out_manifest_path, plugin_path.join("manifest.yaml")).unwrap();
    let lib_name = format!("lib{pkg_name}.{LIB_EXT}");
    std::os::unix::fs::symlink(out_dir.join(&lib_name), plugin_path.join(lib_name)).unwrap();

    if !migrations.is_empty() {
        std::os::unix::fs::symlink(out_dir.join("migrations"), plugin_path.join("migrations"))
            .unwrap();

        for m in &migrations {
            println!("cargo::rerun-if-changed={m}");
        }
    }

    println!("cargo::rerun-if-changed={MANIFEST_TEMPLATE_NAME}");
}
