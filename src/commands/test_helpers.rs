//! Helpers shared by unit tests of several commands.

use crate::commands::run::{Tier, Topology};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(target_os = "linux")]
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
};

/// Removed with everything inside on drop, so a failed assertion leaves nothing in `/tmp`.
pub(crate) struct UniqueTempDirRemovedOnDrop(PathBuf);

impl UniqueTempDirRemovedOnDrop {
    pub(crate) fn create(prefix: &str) -> Self {
        let nanos_since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let unique_temp_dir =
            std::env::temp_dir().join(format!("pike-ut-{prefix}-{nanos_since_epoch}"));
        fs::create_dir_all(&unique_temp_dir).unwrap();
        Self(unique_temp_dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for UniqueTempDirRemovedOnDrop {
    fn drop(&mut self) {
        // The dir may be removed by the test itself; there is nothing to clean up then.
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A long-running `sh` with `cmdline_args` in its command line, as `/proc` shows an instance
/// started by `pike run`. The wait matters: `spawn` may return while the child is still
/// inside `exec`, with an empty command line in `/proc`.
#[cfg(target_os = "linux")]
pub(crate) fn spawn_fake_picodata_instance_and_wait_until_started(
    cwd: &Path,
    cmdline_args: &[&str],
) -> Child {
    // The shell prints it once it runs, so once its command line is in `/proc`.
    let started_signal = "started";
    // `cmdline_args` become positional parameters, which the script ignores. The builtin
    // `read` keeps the shell alive with no child process until it is killed or its stdin
    // is closed with the `Child`.
    let mut child = Command::new("sh")
        .args(["-c", &format!("echo {started_signal}; read _"), "sh"])
        .args(cmdline_args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut started_line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut started_line)
        .unwrap();
    assert_eq!(started_line.trim_end(), started_signal);
    child
}

/// `tier_sizes` holds `(tier name, replicasets, replication_factor)`.
pub(crate) fn make_topology_with_tier_sizes(tier_sizes: &[(&str, u8, u8)]) -> Topology {
    Topology {
        tiers: tier_sizes
            .iter()
            .map(|&(tier_name, replicasets, replication_factor)| {
                (
                    tier_name.to_owned(),
                    Tier {
                        replicasets,
                        replication_factor,
                    },
                )
            })
            .collect(),
        ..Topology::default()
    }
}
