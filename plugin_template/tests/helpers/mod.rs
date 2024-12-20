use log::info;
use std::{
	fs::{self},
	io::ErrorKind,
	path::{Path, PathBuf},
};

// TODO: check in workspaces
pub const TMP_DIR: &str = "tmp/";
pub const TOPOLOGY_PATH: &str = "topology.toml";
pub const TARGET_DIR: &str = "target";

pub struct Cluster {}

impl Drop for Cluster {
	fn drop(&mut self) {
		let data_dir = PathBuf::from(TMP_DIR.to_owned());
		pike::cluster::stop(&data_dir).unwrap();
	}
}

impl Cluster {
	pub fn new() -> Cluster {
		info!("cleaning artefacts from previous run");

		match fs::remove_file(Path::new(TMP_DIR).join("instance.log")) {
			Ok(()) => info!("Clearing logs."),
			Err(e) if e.kind() == ErrorKind::NotFound => {
				info!("instance.log not found, skipping cleanup");
			},
			Err(e) => panic!("failed to delete instance.log: {e}"),
		}

		match fs::remove_dir_all(TMP_DIR) {
			Ok(()) => info!("clearing test plugin dir."),
			Err(e) if e.kind() == ErrorKind::NotFound => {
				info!("plugin dir not found, skipping cleanup");
			},
			Err(e) => panic!("failed to delete plugin_dir: {e}"),
		}

		Cluster {}
	}
}

pub fn run_cluster() -> Cluster {
	let cluster_handle = Cluster::new();
	let data_dir = PathBuf::from(TMP_DIR.to_owned());
	let topology_path = PathBuf::from(TOPOLOGY_PATH.to_owned());
	let target_dir = PathBuf::from(TARGET_DIR.to_owned());
	pike::cluster::run(
		&topology_path,
		&data_dir,
		false,
		8000,
		&PathBuf::from("picodata".to_owned()),
		5432,
		false,
        &target_dir,
	)
	.unwrap();
	cluster_handle
}
