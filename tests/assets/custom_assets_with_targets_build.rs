use pike::helpers::build;

fn main() {
    let params = build::ParamsBuilder::default()
        .custom_assets_with_targets(vec![
            ("Cargo.toml", "not.cargo"),
            ("src", "other/name"),
            ("Cargo.lock", "other/name/Cargo.unlock"),
        ])
        .custom_assets(vec!["Cargo.toml"])
        .build()
        .unwrap();
    build::main(&params);
}
