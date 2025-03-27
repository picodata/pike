use pike::helpers::build;

// Custom build script for plugin, which stores
// plugin's artefacts in corresponding folder
//
// Call of build::main() function is MANDATORY
// for proper artefact storage and packing

fn main() {
    let params = build::ParamsBuilder::default()
        .custom_assets(vec!["plugin_config.yaml"])
        .build()
        .unwrap();
    build::main(&params);
}
