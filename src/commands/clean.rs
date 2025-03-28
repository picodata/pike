use crate::commands;
use anyhow::{Context, Result};
use log::{info, warn};
use std::{fs, path::Path};

pub fn cmd(data_dir: &Path) -> Result<()> {
    info!("Clearing cluster data directory:");
    let params = commands::stop::ParamsBuilder::default()
        .data_dir(data_dir.into())
        .build()
        .unwrap();
    let _ = commands::stop::cmd(&params).context("failed stop cluster before clean");
    if data_dir.exists() {
        fs::remove_dir_all(data_dir)
            .context(format!("failed to remove directory {}", data_dir.display()))?;
        info!("Successfully removed : {}", data_dir.to_string_lossy());
    } else {
        warn!("Data directory does not exist");
    }

    Ok(())
}
