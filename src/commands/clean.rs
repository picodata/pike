use crate::commands;
use anyhow::{Context, Result};
use log::{info, warn};
use std::{fs, path::Path};

pub fn cmd(data_dir: &Path, plugin_path: &Path) -> Result<()> {
    info!("Clearing cluster data directory:");
    let params = commands::stop::ParamsBuilder::default()
        .data_dir(data_dir.into())
        .plugin_path(plugin_path.into())
        .build()
        .unwrap();
    let _ = commands::stop::cmd(&params).context("failed stop cluster before clean");

    let plugin_data_dir = plugin_path.join(data_dir);
    if plugin_data_dir.exists() {
        fs::remove_dir_all(&plugin_data_dir).context(format!(
            "failed to remove directory {}",
            plugin_data_dir.display()
        ))?;
        info!(
            "Successfully removed : {}",
            plugin_data_dir.to_string_lossy()
        );
    } else {
        warn!("Data directory does not exist");
    }

    Ok(())
}
