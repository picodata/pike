use crate::commands::lib::{get_active_socket_path, get_cluster_dir};
use anyhow::{bail, Context, Result};
use colored::Colorize;
use derive_builder::Builder;
use log::{info, warn};
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::fs::{self};
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

// Default signal sent by unix kill command during `pike stop` command.
pub const DEFAULT_STOP_SIGNAL: Signal = Signal::SIGKILL;
// Default timeout for graceful process shutdown.
pub const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Builder)]
pub struct Params {
    #[builder(default = "PathBuf::from(\"./tmp\")")]
    data_dir: PathBuf,
    #[builder(default = "PathBuf::from(\"./\")")]
    plugin_path: PathBuf,
    #[builder(default)]
    instance_name: Option<String>,
    #[builder(default = DEFAULT_STOP_SIGNAL)]
    signal: Signal,
    #[builder(default = DEFAULT_STOP_TIMEOUT)]
    timeout: Duration,
}

pub fn cmd(params: &Params) -> Result<()> {
    let instances_path = params.plugin_path.join(params.data_dir.join("cluster"));
    let dirs = fs::read_dir(&instances_path).context(format!(
        "cluster data dir with path {} does not exist",
        instances_path.to_string_lossy()
    ))?;

    let Params {
        data_dir,
        plugin_path,
        instance_name,
        signal,
        timeout,
    } = params;

    let cluster_dir = get_cluster_dir(plugin_path, data_dir);

    if let Some(instance_name) = instance_name {
        info!(
            "stopping picodata cluster instance '{instance_name}', data folder: {}",
            instances_path.join(instance_name).to_string_lossy()
        );

        // Find directory that belongs to stopping instance.
        let instance_dir = dirs.into_iter().find_map(|result| {
            let Ok(dir_entry) = result else {
                return None;
            };

            if dir_entry.file_name() != instance_name.as_str() {
                return None;
            }

            Some(dir_entry.path())
        });

        let Some(instance_dir) = instance_dir else {
            bail!("failed to locate directory of the instance '{instance_name}'");
        };

        stop_instance(&cluster_dir, &instance_dir, *signal, *timeout)
    } else {
        info!(
            "stopping picodata cluster, data folder: {}",
            params.data_dir.to_string_lossy()
        );

        // Iterate through instance folders and collect
        // them into vector.
        let mut instance_dirs = vec![];

        for current_dir in dirs {
            let instance_dir = current_dir?.path();

            // Skip symlinks.
            if fs::symlink_metadata(&instance_dir)?.is_symlink() {
                continue;
            }

            instance_dirs.push(instance_dir);
        }

        if instance_dirs.is_empty() {
            info!("cluster is empty");
            return Ok(());
        }

        info!(
            "stopping {} instance(s) (timeout = {timeout:?})",
            instance_dirs.len()
        );

        // Calculate timeout per picodata process.
        let start = Instant::now();
        let timeout_per_instance = *timeout / u32::try_from(instance_dirs.len())?;

        // Iterate over instances and stop them one-by-one.
        for instance_dir in instance_dirs {
            stop_instance(&cluster_dir, &instance_dir, *signal, timeout_per_instance)?;
        }

        info!("cluster stopped in {:?}", start.elapsed());

        Ok(())
    }
}

fn stop_instance(
    cluster_dir: &Path,
    instance_dir: &Path,
    signal: Signal,
    timeout: Duration,
) -> Result<()> {
    if !instance_dir.is_dir() {
        bail!("{} is not a directory", instance_dir.to_string_lossy());
    }
    let Some(link_name) = instance_dir.file_name() else {
        bail!(
            "failed to obtain filename from {}",
            instance_dir.to_string_lossy()
        );
    };

    let pid_file_path = instance_dir.join("pid");
    if !pid_file_path.exists() {
        bail!(
            "PID file does not exist in folder: {}",
            instance_dir.display()
        );
    }

    let pid = read_pid_from_file(&pid_file_path).context("failed to read the PID file")?;

    if get_active_socket_path(cluster_dir, link_name.to_str().unwrap()).is_none() {
        info!(
            "stopping picodata instance: {} - {}",
            link_name.to_string_lossy(),
            "SKIPPED".yellow()
        );
        return Ok(());
    }

    if let Err(e) = send_signal_and_wait(pid, signal, timeout) {
        bail!("failed to stop picodata instance with PID {pid}. Error: {e}");
    }

    info!(
        "stopping picodata instance: {} - {}",
        link_name.to_string_lossy(),
        "OK".green()
    );

    Ok(())
}

fn read_pid_from_file(pid_file_path: &Path) -> Result<Pid> {
    let file = fs::File::open(pid_file_path)?;

    let mut lines = io::BufReader::new(file).lines();
    let pid_line = lines.next().context("PID file is empty")??;

    let pid = pid_line.trim().parse::<i32>().context(format!(
        "failed to parse PID from file {}",
        pid_file_path.display()
    ))?;

    assert!(pid > 0);

    Ok(Pid::from_raw(pid))
}

/// Send signal to process and wait until it exits.
///
/// If the process does not terminate within the specified timeout,
/// SIGKILL is sent as a fallback.
///
/// On Linux the wait is on a pidfd, which turns readable only once every
/// thread of the process has exited: that is when its files, and the locks
/// on them, are released. The state in `/proc/<pid>/stat` is the main
/// thread's, which can be a zombie while other threads still hold the
/// instance's data directory, so a `pike run` right after the stop would
/// fail to lock it. A kernel without pidfds (before 5.3) gets the polling
/// below.
#[cfg(target_os = "linux")]
fn send_signal_and_wait(pid: Pid, signal: Signal, timeout: Duration) -> anyhow::Result<()> {
    use std::os::fd::{FromRawFd, OwnedFd};

    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid.as_raw(), 0) };
    if fd < 0 {
        return match Errno::last() {
            Errno::ESRCH => Ok(()),
            Errno::ENOSYS => send_signal_and_poll(pid, signal, timeout),
            err => bail!(err),
        };
    }
    let fd = libc::c_int::try_from(fd).context("pidfd_open returned no descriptor")?;
    // SAFETY: the descriptor was just opened and nothing else owns it.
    let pidfd = unsafe { OwnedFd::from_raw_fd(fd) };

    pidfd_send_signal(&pidfd, signal)?;
    if pidfd_exited(&pidfd, timeout)? {
        return Ok(());
    }

    warn!("Process {pid} did not terminate within {timeout:?}. Sending SIGKILL...");
    pidfd_send_signal(&pidfd, Signal::SIGKILL)?;
    if !pidfd_exited(&pidfd, timeout)? {
        bail!("process {pid} did not exit within {timeout:?} of SIGKILL");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn send_signal_and_wait(pid: Pid, signal: Signal, timeout: Duration) -> anyhow::Result<()> {
    send_signal_and_poll(pid, signal, timeout)
}

/// Signal the process the pidfd names, and no later one with its pid.
#[cfg(target_os = "linux")]
fn pidfd_send_signal(pidfd: &std::os::fd::OwnedFd, signal: Signal) -> anyhow::Result<()> {
    use std::os::fd::AsRawFd;

    // SAFETY: a valid pidfd, a signal number, no siginfo and no flags.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal as libc::c_int,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    // ESRCH: the process has exited already, which is what the caller waits for.
    if rc < 0 && Errno::last() != Errno::ESRCH {
        bail!(Errno::last());
    }
    Ok(())
}

/// Whether the process behind the pidfd exits, all its threads, within `timeout`.
#[cfg(target_os = "linux")]
fn pidfd_exited(pidfd: &std::os::fd::OwnedFd, timeout: Duration) -> anyhow::Result<bool> {
    use std::os::fd::AsRawFd;

    let deadline = Instant::now() + timeout;
    let mut fds = [libc::pollfd {
        fd: pidfd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    }];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ms = libc::c_int::try_from(left.as_millis()).unwrap_or(libc::c_int::MAX);
        // SAFETY: one valid pollfd.
        match unsafe { libc::poll(fds.as_mut_ptr(), 1, ms) } {
            0 => return Ok(false),
            n if n > 0 => return Ok(true),
            _ => {
                let err = Errno::last();
                if err != Errno::EINTR {
                    bail!(err);
                }
            }
        }
    }
}

/// [`send_signal_and_wait`] without a pidfd: poll until the process is gone
/// or, on Linux, its main thread is a zombie.
fn send_signal_and_poll(pid: Pid, signal: Signal, timeout: Duration) -> anyhow::Result<()> {
    kill(pid, signal)?;

    let start = Instant::now();
    let delay = Duration::from_millis(100);

    while start.elapsed() < timeout {
        sleep(delay);

        match kill(pid, None) {
            Err(Errno::ESRCH) => {
                // Process no longer exists.
                return Ok(());
            }
            Err(err) => bail!(err),
            Ok(()) => {}
        }

        #[cfg(target_os = "linux")]
        if is_zombie(pid) {
            return Ok(());
        }
    }

    warn!("Process {pid} did not terminate within {timeout:?}. Sending SIGKILL...");
    kill(pid, Signal::SIGKILL)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_zombie(pid: Pid) -> bool {
    use procfs::process::{ProcState, Process};
    // Returns true if the process doesn't exist or we can't read /proc.
    Process::new(pid.into())
        .and_then(|p| p.stat())
        .and_then(|s| s.state())
        // Process is terminated, but not reaped by parent.
        // Or we can't check process state or there is not such process in /proc.
        .map_or(true, |state| state == ProcState::Zombie)
}
