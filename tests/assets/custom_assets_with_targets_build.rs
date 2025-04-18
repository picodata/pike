use pike::helpers::build;

fn main() {
    let params = build::ParamsBuilder::default()
        .custom_assets_with_targets([
            ("Cargo.toml", "not.cargo"),
            ("src", "other/name"),
            ("Cargo.lock", "other/name/Cargo.unlock"),
        ])
        .custom_assets(["plugin_config.yaml"])
        .build()
        .unwrap();
    build::main(&params);
}
