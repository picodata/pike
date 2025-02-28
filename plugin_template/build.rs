use pike::helpers::build;

// Custom build script for plugin, which stores
// plugin's artefacts in corresponding folder
//
// Call of build::main() function is MANDATORY
// for proper artefact storage and packing

fn main() {
    // In case you want to store custom files in `assets` folder,
    // params could be initialised like
    //
    // let params = build::ParamsBuilder::default()
    //    .custom_assets(vec!["path_to_file.txt", "another_file.txt"])...
    //
    // The path is calculated from plugin directory

    let params = build::ParamsBuilder::default().build().unwrap();
    build::main(&params);
}
