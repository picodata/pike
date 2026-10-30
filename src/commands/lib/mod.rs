use crate::helpers::cargo_manifest::MANIFEST_TEMPLATE_NAME;
use anyhow::{bail, Context, Result};
use colored::Colorize;
use flate2::bufread::GzDecoder;
use fs_extra::dir;
use log::info;
#[cfg(target_os = "linux")]
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::fmt::Display;
use std::fs::{self, File, FileType};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use tar::Archive;

pub mod instance_info;
pub mod timeout;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub const LIB_EXT: &str = "so";

#[cfg(target_os = "macos")]
pub const LIB_EXT: &str = "dylib";

pub const DEFAULT_DATA_DIR: &str = "./tmp/pike";

/// Passed by `pike run` to `picodata run`.
pub const PICODATA_INSTANCE_DIR_OPTION: &str = "--instance-dir";

pub fn log_instance_skipped(name: impl Display) {
    info!("{name} - {}", "SKIPPED".yellow());
}

pub fn log_instance_started(name: impl Display) {
    info!("{name} - {}", "started".green());
}

#[derive(Clone, Copy, Debug)]
pub enum BuildType {
    Release,
    Debug,
}

impl Display for BuildType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            BuildType::Release => "release",
            BuildType::Debug => "debug",
        })
    }
}

pub fn is_plugin_dir(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    if !path.join("Cargo.toml").exists() {
        return false;
    }

    if path.join(MANIFEST_TEMPLATE_NAME).exists() {
        return true;
    }

    fs::read_dir(path)
        .unwrap()
        .filter(Result::is_ok)
        .map(|e| e.unwrap().path())
        .filter(|e| e.is_dir())
        .any(|dir| dir.join(MANIFEST_TEMPLATE_NAME).exists())
}

pub fn is_plugin_shipping_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!("path is not a plugin shipping directory");
    }
    let versioned_readers = fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|version| version.file_type().as_ref().is_ok_and(FileType::is_dir))
        .map(|path| fs::read_dir(path.path()));
    for version in versioned_readers.flatten() {
        let root_files = version.filter_map(Result::ok).collect::<Vec<_>>();
        let has_manifest = root_files.iter().any(|path| {
            path.file_name() == "manifest.yaml"
                && path.file_type().as_ref().is_ok_and(FileType::is_file)
        });
        if has_manifest {
            return Ok(());
        }
    }
    bail!("path does not match plugin dir structure")
}

/// Checks if provided path contains valid packed plugin archive
pub fn is_plugin_archive(test_path: &Path) -> Result<()> {
    if !test_path.is_file() {
        bail!("plugin archive path must be a file");
    }
    let file = File::options()
        .read(true)
        .write(false)
        .create(false)
        .open(test_path)
        .context("unable to open plugin archive candidate")?;
    let buf_reader = BufReader::new(file);
    let file_untar = GzDecoder::new(buf_reader);
    let mut archive = Archive::new(file_untar);
    let Ok(archive_entries) = archive.entries() else {
        bail!("unable to read plugin archive candidate");
    };
    let mut has_manifest = false;
    let mut has_lib = false;
    let lib_suffix = format!(".{LIB_EXT}");
    for entry in archive_entries.filter_map(Result::ok) {
        if let Ok(entry_path) = entry.path() {
            // plugin_name / plugin_version / root_file_name
            if entry_path.components().count() == 3 {
                if let Some(last_part) = entry_path.components().next_back() {
                    has_manifest = has_manifest || last_part.as_os_str() == "manifest.yaml";
                    has_lib = has_lib
                        || last_part
                            .as_os_str()
                            .to_string_lossy()
                            .ends_with(&lib_suffix);
                }
            }
        }
        if has_manifest && has_lib {
            return Ok(());
        }
    }
    if !has_manifest {
        bail!("plugin archive candidate missing manifest");
    }
    if !has_lib {
        bail!("plugin archive candidate missing plugin library");
    }
    bail!("plugin archive candidate has invalid structure");
}

#[allow(clippy::needless_pass_by_value)]
pub fn cargo_build(build_type: BuildType, target_dir: &PathBuf, build_dir: &PathBuf) -> Result<()> {
    let mut args = vec!["build"];
    if let BuildType::Release = build_type {
        args.push("--release");
    }

    let mut child = Command::new("cargo")
        .args(args)
        .arg("--target-dir")
        .arg(target_dir)
        .stdout(Stdio::piped())
        .current_dir(build_dir)
        .spawn()
        .context("running cargo build")?;

    let stdout = child.stdout.take().expect("Failed to capture stdout");
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = line.unwrap_or_else(|e| format!("{e}"));
        print!("{line}");
    }

    if !child.wait().unwrap().success() {
        let mut stderr = String::new();
        child.stderr.unwrap().read_to_string(&mut stderr).unwrap();
        bail!("build error: {stderr}");
    }

    Ok(())
}

pub fn get_cluster_dir(plugin_path: &Path, data_dir: &Path) -> PathBuf {
    plugin_path.join(data_dir).join("cluster")
}

// Return socket path to the active instance
pub fn get_active_socket_path(cluster_dir: &Path, instance_name: &str) -> Option<PathBuf> {
    let socket_path = cluster_dir.join(instance_name).join("admin.sock");

    if socket_path.exists() && UnixStream::connect(&socket_path).is_ok() {
        return Some(socket_path);
    }

    None
}

// Scan directory with instances and return the first active socket path
pub fn find_active_socket_path(cluster_dir: &Path) -> Result<Option<PathBuf>> {
    if !cluster_dir.exists() {
        return Ok(None);
    }

    let dirs = fs::read_dir(cluster_dir).context(format!(
        "cluster data dir with path {} does not exist",
        cluster_dir.display()
    ))?;

    for current_dir in dirs {
        let dir_name = current_dir?.file_name();
        if let Some(name) = dir_name.to_str() {
            let socket_path = get_active_socket_path(cluster_dir, name);
            if socket_path.is_some() {
                return Ok(socket_path);
            }
        }
    }

    Ok(None)
}

/// Validates and unpacks plugin(s) from shipping archive into destination path,
/// preserving archive structure. Does not create destination path itself.
pub fn unpack_shipping_archive(src_path: &Path, dst_path: &Path) -> Result<()> {
    is_plugin_archive(src_path).with_context(|| {
        let (from, to) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("can not unpack shipping archive at {from} to {to}")
    })?;

    let file = File::options()
        .read(true)
        .write(false)
        .create(false)
        .open(src_path)
        .context("unable to open plugin archive")?;
    let buf_reader = BufReader::new(file);
    let decompressor = GzDecoder::new(buf_reader);

    // by default - override existing, preserve mtime
    let mut archive = Archive::new(decompressor);
    archive.unpack(dst_path).with_context(|| {
        let (from, to) = (src_path.to_string_lossy(), dst_path.to_string_lossy());
        format!("failed to unpack shipping archive at {from} to {to}")
    })?;
    Ok(())
}

/// Copies directory at `src_path` into `dst_dir`
pub fn copy_directory_tree(src_path: &Path, dst_dir: &Path) -> Result<()> {
    let src_path = src_path.canonicalize().with_context(|| {
        let src_path = src_path.to_string_lossy();
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("<unknown>"));
        let current_dir = current_dir.to_string_lossy();
        format!("path {src_path} does not exists or not a directory (pwd {current_dir})")
    })?;
    let opts = dir::CopyOptions::default().overwrite(true);
    dir::copy(&src_path, dst_dir, &opts).with_context(|| {
        let (src_path, dst_path) = (src_path.to_string_lossy(), dst_dir.to_string_lossy());
        format!("failed to copy directory tree from {src_path} to {dst_path}")
    })?;
    Ok(())
}

/// Spawns picodata admin in a new process.
pub fn spawn_picodata_admin(picodata_path: &Path, socket_path: &Path) -> Result<Child> {
    Command::new(picodata_path)
        .arg("admin")
        .arg(socket_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn child proccess of picodata admin")
}

/// Sends text to admin.sock and returns received stdout.
pub fn run_query_in_picodata_admin(
    picodata_path: &Path,
    socket_path: &Path,
    query: &str,
) -> Result<String> {
    run_query_in_picodata_admin_with_timeout(picodata_path, socket_path, query, None)
}

/// Sends text to admin.sock and returns received stdout.
///
/// With `Some(timeout)` the `picodata admin` process is killed if it does not exit within
/// `timeout`, and the error `no response from <socket> within <secs>s` is returned.
pub fn run_query_in_picodata_admin_with_timeout(
    picodata_path: &Path,
    socket_path: &Path,
    query: &str,
    timeout: Option<Duration>,
) -> Result<String> {
    let mut picodata_admin = spawn_picodata_admin(picodata_path, socket_path)?;
    {
        // Dropped at the end of the block: closing stdin lets admin see the end of the query.
        let mut picodata_stdin = picodata_admin.stdin.take().unwrap();
        match picodata_stdin.write_all(query.as_bytes()) {
            // Admin has exited without reading the query: its exit code and stderr explain why.
            Err(err) if err.kind() == ErrorKind::BrokenPipe => {}
            query_write_result => {
                query_write_result.context("failed to send text in admin socket")?;
            }
        }
    }

    // `wait_with_output` reads stdout and stderr while waiting, so admin never blocks
    // on a full pipe.
    let picodata_admin_output = match timeout {
        None => picodata_admin
            .wait_with_output()
            .context("failed to wait for picodata admin")?,
        Some(timeout) => wait_for_picodata_admin_output_or_kill_it_on_timeout(
            picodata_admin,
            socket_path,
            timeout,
        )?,
    };

    if !picodata_admin_output.status.success() {
        let stderr = String::from_utf8(picodata_admin_output.stderr)
            .context("failed to read stderr of picodata admin child")?;
        bail!("failed to run query in picodata admin: {stderr}");
    }

    String::from_utf8(picodata_admin_output.stdout)
        .context("failed to read stdout of picodata admin child")
}

/// `wait_with_output` blocks, so a separate thread owns the child and sends the result over
/// a channel. That thread may collect the exit status of the child and free its PID at any
/// moment, so on Linux `SIGKILL` goes through a pidfd and never reaches a process that reused
/// the PID. After `SIGKILL` the thread collects the exit status; waiting up to 1 s for that
/// leaves no zombie behind.
fn wait_for_picodata_admin_output_or_kill_it_on_timeout(
    picodata_admin: Child,
    socket_path: &Path,
    timeout: Duration,
) -> Result<Output> {
    let picodata_admin_pid = Pid::from_raw(
        i32::try_from(picodata_admin.id()).context("pid of picodata admin does not fit i32")?,
    );
    // Opened before the thread below may collect the exit status of the child. A kernel without
    // pidfds (before 5.3) gets the plain `kill` by PID.
    #[cfg(target_os = "linux")]
    let picodata_admin_pidfd = open_pidfd(picodata_admin_pid).ok();
    let (output_sender, output_receiver) = mpsc::channel();
    thread::spawn(move || {
        // The receiver is gone only after a timeout, when nobody needs the result.
        let _ = output_sender.send(picodata_admin.wait_with_output());
    });

    match output_receiver.recv_timeout(timeout) {
        Ok(picodata_admin_output) => {
            picodata_admin_output.context("failed to wait for picodata admin")
        }
        Err(RecvTimeoutError::Timeout) => {
            // The child may exit right at the deadline: the signal then fails with `ESRCH`,
            // which is harmless.
            #[cfg(target_os = "linux")]
            let _ = match &picodata_admin_pidfd {
                Some(picodata_admin_pidfd) => {
                    pidfd_send_signal(picodata_admin_pidfd, Signal::SIGKILL)
                }
                None => kill(picodata_admin_pid, Signal::SIGKILL).map_err(anyhow::Error::from),
            };
            #[cfg(not(target_os = "linux"))]
            let _ = kill(picodata_admin_pid, Signal::SIGKILL);
            let _ = output_receiver.recv_timeout(Duration::from_secs(1));
            bail!(
                "no response from {} within {}s",
                socket_path.display(),
                timeout.as_secs()
            );
        }
        Err(RecvTimeoutError::Disconnected) => {
            bail!("thread waiting for picodata admin exited without a result")
        }
    }
}

/// Opens a pidfd of a process whose exit status is not collected yet: signals sent through it
/// never reach a later process with the same PID.
#[cfg(target_os = "linux")]
pub fn open_pidfd(pid: Pid) -> Result<std::os::fd::OwnedFd, Errno> {
    use std::os::fd::{FromRawFd, OwnedFd};

    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid.as_raw(), 0) };
    if fd < 0 {
        return Err(Errno::last());
    }
    let fd = libc::c_int::try_from(fd).map_err(|_| Errno::EBADF)?;
    // SAFETY: the descriptor was just opened and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Signal the process the pidfd names, and no later one with its pid.
#[cfg(target_os = "linux")]
pub fn pidfd_send_signal(pidfd: &std::os::fd::OwnedFd, signal: Signal) -> anyhow::Result<()> {
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

/// Reads the PID from the first line of `pid_file_path`.
///
/// The sign of the PID is not checked: callers decide what a non-positive PID means.
pub fn read_pid_file(pid_file_path: &Path) -> Result<Pid> {
    let file = File::open(pid_file_path)?;

    let mut lines = BufReader::new(file).lines();
    let pid_line = lines.next().context("PID file is empty")??;

    let pid = pid_line.trim().parse::<i32>().context(format!(
        "failed to parse PID from file {}",
        pid_file_path.display()
    ))?;

    Ok(Pid::from_raw(pid))
}

#[cfg(target_os = "linux")]
pub fn is_zombie(pid: Pid) -> bool {
    use procfs::process::{ProcState, Process};
    // Returns true if the process doesn't exist or we can't read /proc.
    Process::new(pid.into())
        .and_then(|p| p.stat())
        .and_then(|s| s.state())
        // Process is terminated, but not reaped by parent.
        // Or we can't check process state or there is not such process in /proc.
        .map_or(true, |state| state == ProcState::Zombie)
}

/// Canonical path; `None` if the process is gone, `/proc` is unreadable, there is no
/// `--instance-dir` or its dir does not exist.
#[cfg(target_os = "linux")]
pub fn resolve_instance_dir_of_process_from_cmdline_and_cwd(pid: Pid) -> Option<PathBuf> {
    let procfs_process = procfs::process::Process::new(pid.as_raw()).ok()?;
    let process_cmdline_args = procfs_process.cmdline().ok()?;
    let instance_dir_option_value = find_instance_dir_option_value(&process_cmdline_args)?;
    procfs_process
        .cwd()
        .ok()?
        .join(instance_dir_option_value)
        .canonicalize()
        .ok()
}

/// Accepts `--instance-dir <dir>` and `--instance-dir=<dir>`.
#[cfg(target_os = "linux")]
fn find_instance_dir_option_value(cmdline_args: &[String]) -> Option<&str> {
    let mut remaining_cmdline_args = cmdline_args.iter();
    while let Some(cmdline_arg) = remaining_cmdline_args.next() {
        if cmdline_arg == PICODATA_INSTANCE_DIR_OPTION {
            return remaining_cmdline_args.next().map(String::as_str);
        }
        if let Some(instance_dir_option_value) = cmdline_arg
            .strip_prefix(PICODATA_INSTANCE_DIR_OPTION)
            .and_then(|after_option_name| after_option_name.strip_prefix('='))
        {
            return Some(instance_dir_option_value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use crate::commands::test_helpers::spawn_fake_picodata_instance_and_wait_until_started;
    use crate::commands::test_helpers::UniqueTempDirRemovedOnDrop;
    use std::time::{Duration, Instant};

    fn write_temp_pid_file_and_read_it(prefix: &str, pid_file_contents: &str) -> Result<Pid> {
        let temp_dir = UniqueTempDirRemovedOnDrop::create(prefix);
        let pid_file_path = temp_dir.path().join("pid");
        fs::write(&pid_file_path, pid_file_contents).unwrap();
        read_pid_file(&pid_file_path)
    }

    #[test]
    fn read_pid_file_reads_first_line() {
        assert_eq!(
            write_temp_pid_file_and_read_it("first-line", "4242\n").unwrap(),
            Pid::from_raw(4242)
        );
        assert_eq!(
            write_temp_pid_file_and_read_it("trimmed", "  17  \nmore").unwrap(),
            Pid::from_raw(17)
        );
    }

    #[test]
    fn read_pid_file_keeps_sign_check_to_callers() {
        assert_eq!(
            write_temp_pid_file_and_read_it("zero", "0").unwrap(),
            Pid::from_raw(0)
        );
        assert_eq!(
            write_temp_pid_file_and_read_it("negative", "-1").unwrap(),
            Pid::from_raw(-1)
        );
    }

    #[test]
    fn read_pid_file_rejects_garbage() {
        let err = write_temp_pid_file_and_read_it("empty", "").unwrap_err();
        assert!(format!("{err:#}").contains("PID file is empty"), "{err:#}");

        let err = write_temp_pid_file_and_read_it("garbage", "abc").unwrap_err();
        assert!(
            format!("{err:#}").contains("failed to parse PID"),
            "{err:#}"
        );

        let temp_dir = UniqueTempDirRemovedOnDrop::create("missing-pid");
        assert!(read_pid_file(&temp_dir.path().join("pid")).is_err());
    }

    #[cfg(target_os = "linux")]
    fn spawn_fake_picodata_instance_read_its_instance_dir_and_kill_it(
        cwd: &Path,
        instance_dir_args: &[&str],
    ) -> Option<PathBuf> {
        let mut child = spawn_fake_picodata_instance_and_wait_until_started(cwd, instance_dir_args);
        let resolved_instance_dir = resolve_instance_dir_of_process_from_cmdline_and_cwd(
            Pid::from_raw(i32::try_from(child.id()).unwrap()),
        );
        child.kill().unwrap();
        child.wait().unwrap();
        resolved_instance_dir
    }

    #[cfg(target_os = "linux")]
    fn create_temp_cluster_dir_with_i1(prefix: &str) -> UniqueTempDirRemovedOnDrop {
        let cluster_dir = UniqueTempDirRemovedOnDrop::create(prefix);
        fs::create_dir(cluster_dir.path().join("i1")).unwrap();
        cluster_dir
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_instance_dir_of_process_resolves_option_against_cwd() {
        let cluster_dir = create_temp_cluster_dir_with_i1("instance-dir");
        let expected_instance_dir = cluster_dir.path().join("i1").canonicalize().unwrap();

        // `pike run` passes a relative path like `././tmp/pike/cluster/i1`.
        let instance_dir_from_separate_option =
            spawn_fake_picodata_instance_read_its_instance_dir_and_kill_it(
                cluster_dir.path(),
                &[PICODATA_INSTANCE_DIR_OPTION, "././i1"],
            );
        let instance_dir_from_joined_option =
            spawn_fake_picodata_instance_read_its_instance_dir_and_kill_it(
                cluster_dir.path(),
                &[&format!("{PICODATA_INSTANCE_DIR_OPTION}=./i1")],
            );

        assert_eq!(
            instance_dir_from_separate_option,
            Some(expected_instance_dir.clone())
        );
        assert_eq!(instance_dir_from_joined_option, Some(expected_instance_dir));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_instance_dir_of_process_without_option_or_dir_is_none() {
        let cluster_dir = create_temp_cluster_dir_with_i1("no-instance-dir");

        let instance_dir_without_option =
            spawn_fake_picodata_instance_read_its_instance_dir_and_kill_it(cluster_dir.path(), &[]);
        let instance_dir_of_missing_dir =
            spawn_fake_picodata_instance_read_its_instance_dir_and_kill_it(
                cluster_dir.path(),
                &[PICODATA_INSTANCE_DIR_OPTION, "./i9"],
            );

        assert_eq!(instance_dir_without_option, None);
        assert_eq!(instance_dir_of_missing_dir, None);
        // The test binary is not started with `--instance-dir`.
        assert_eq!(
            resolve_instance_dir_of_process_from_cmdline_and_cwd(Pid::this()),
            None
        );
    }

    /// The script is invoked as `<script> admin <socket>` and gets the query on stdin.
    fn create_fake_picodata_script_in_unique_temp_dir(
        prefix: &str,
        script_body: &str,
    ) -> (UniqueTempDirRemovedOnDrop, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let fake_picodata_dir = UniqueTempDirRemovedOnDrop::create(prefix);
        let fake_picodata_script = fake_picodata_dir.path().join("picodata");
        fs::write(&fake_picodata_script, format!("#!/bin/sh\n{script_body}\n")).unwrap();
        fs::set_permissions(&fake_picodata_script, fs::Permissions::from_mode(0o755)).unwrap();
        (fake_picodata_dir, fake_picodata_script)
    }

    /// A process spawned by a parallel test may briefly inherit the write descriptor of a
    /// freshly written script, and `exec` of that script then fails with `ETXTBSY`: such a spawn
    /// is retried.
    fn run_query_through_fake_picodata_retrying_busy_script(
        fake_picodata_script: &Path,
        timeout: Option<Duration>,
    ) -> Result<String> {
        let fake_admin_socket_path = fake_picodata_script.with_file_name("admin.sock");
        for _ in 0..10 {
            let query_result = run_query_in_picodata_admin_with_timeout(
                fake_picodata_script,
                &fake_admin_socket_path,
                "\\lua\nreturn 1",
                timeout,
            );
            let is_script_busy = query_result.as_ref().is_err_and(|err| {
                err.chain()
                    .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
                    .any(|io_err| io_err.kind() == ErrorKind::ExecutableFileBusy)
            });
            if !is_script_busy {
                return query_result;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "fake picodata {} stays busy",
            fake_picodata_script.display()
        );
    }

    #[test]
    fn admin_query_returns_stdout() {
        let (_fake_picodata_dir, fake_picodata_script) =
            create_fake_picodata_script_in_unique_temp_dir(
                "admin-stdout",
                "cat >/dev/null; echo '- ok'",
            );

        assert_eq!(
            run_query_through_fake_picodata_retrying_busy_script(&fake_picodata_script, None)
                .unwrap(),
            "- ok\n"
        );
        assert_eq!(
            run_query_through_fake_picodata_retrying_busy_script(
                &fake_picodata_script,
                Some(Duration::from_secs(5))
            )
            .unwrap(),
            "- ok\n"
        );
        // The old entry point delegates to the same code.
        assert_eq!(
            run_query_in_picodata_admin(
                &fake_picodata_script,
                &fake_picodata_script.with_file_name("admin.sock"),
                "q"
            )
            .unwrap(),
            "- ok\n"
        );
    }

    #[test]
    fn admin_query_fails_with_stderr() {
        let (_fake_picodata_dir, fake_picodata_script) =
            create_fake_picodata_script_in_unique_temp_dir("admin-stderr", "echo boom >&2; exit 1");

        for timeout in [None, Some(Duration::from_secs(5))] {
            let err = run_query_through_fake_picodata_retrying_busy_script(
                &fake_picodata_script,
                timeout,
            )
            .unwrap_err();
            let message = format!("{err:#}");
            assert!(
                message.contains("failed to run query in picodata admin"),
                "{message}"
            );
            assert!(message.contains("boom"), "{message}");
        }
    }

    #[test]
    fn admin_query_times_out_and_kills_child() {
        use nix::errno::Errno;

        let (_fake_picodata_dir, fake_picodata_script) =
            create_fake_picodata_script_in_unique_temp_dir(
                "admin-timeout",
                "echo $$ > \"$(dirname \"$0\")/child.pid\"; exec sleep 30",
            );
        let child_pid_file_path = fake_picodata_script.with_file_name("child.pid");

        let query_started_at = Instant::now();
        // Long enough for `sh` to write its pid before it is killed.
        let err = run_query_through_fake_picodata_retrying_busy_script(
            &fake_picodata_script,
            Some(Duration::from_secs(1)),
        )
        .unwrap_err();
        let query_duration = query_started_at.elapsed();

        let message = format!("{err:#}");
        assert!(message.contains("no response from"), "{message}");
        assert!(
            query_duration < Duration::from_secs(3),
            "{query_duration:?}"
        );

        // The child is killed and its exit status collected: its pid no longer exists. That may
        // finish a bit after the query returns, so wait for it instead of checking once.
        let child_pid = read_pid_file(&child_pid_file_path).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while kill(child_pid, None) != Err(Errno::ESRCH) {
            assert!(
                Instant::now() < deadline,
                "child {child_pid} is still alive"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn admin_query_reads_large_output() {
        let (_fake_picodata_dir, fake_picodata_script) =
            create_fake_picodata_script_in_unique_temp_dir(
                "admin-large",
                "cat >/dev/null; head -c 200000 /dev/zero | tr '\\0' x",
            );

        let stdout =
            run_query_through_fake_picodata_retrying_busy_script(&fake_picodata_script, None)
                .unwrap();
        assert_eq!(stdout.len(), 200_000);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolve_instance_dir_of_missing_process_is_none() {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
        child.wait().unwrap();

        assert_eq!(
            resolve_instance_dir_of_process_from_cmdline_and_cwd(pid),
            None
        );
    }
}
