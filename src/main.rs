use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nix::unistd::{fork, ForkResult};
use std::{env, path::PathBuf, process, thread, time::Duration};

mod commands;

const CK_CHECK_PARRENT_INTERVAL_SEC: u64 = 3;

/// A helper utility to work with Picodata plugins.
#[derive(Parser)]
#[command(
    bin_name = "cargo pike",
    version,
    about,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run Picodata cluster
    Run {
        #[arg(short, long, value_name = "TOPOLOGY", default_value = "topology.toml")]
        topology: PathBuf,
        #[arg(long, value_name = "DATA_DIR", default_value = "./tmp")]
        data_dir: PathBuf,
        /// Disable the automatic installation of plugins
        #[arg(long)]
        disable_install_plugins: bool,
        /// Base http port for picodata instances
        #[arg(long, default_value = "8000")]
        base_http_port: i32,
        /// Port for Pgproto server
        #[arg(long, default_value = "5432")]
        base_pg_port: i32,
        /// Specify path to picodata binary
        #[arg(long, value_name = "BINARY_PATH", default_value = "picodata")]
        picodata_path: PathBuf,
        /// Run release version of plugin
        #[arg(long)]
        release: bool,
        /// Change target folder
        #[arg(long, value_name = "TARGET_DIR", default_value = "target")]
        target_dir: PathBuf,
        // TODO: add demon flag, if true then set output logs to file and release stdin
    },
    /// Stop Picodata cluster
    Stop {
        #[arg(long, value_name = "DATA_DIR", default_value = "./tmp")]
        data_dir: PathBuf,
    },
    /// Remove all data files of previous cluster run
    Clean {
        #[arg(long, value_name = "DATA_DIR", default_value = "./tmp")]
        data_dir: PathBuf,
    },
    /// Helpers for work with plugins
    Plugin {
        #[command(subcommand)]
        command: Plugin,
    },
    /// Helpers for work with config of services
    Config {
        #[command(subcommand)]
        command: Config,
    },
}

#[derive(Subcommand)]
enum Plugin {
    /// Pack your plugin into a distributable bundle
    Pack {
        /// Pack the archive with debug version of plugin
        #[arg(long)]
        debug: bool,
        /// Change target folder
        #[arg(long, value_name = "TARGET_DIR", default_value = "target")]
        target_dir: PathBuf,
    },
    /// Alias for cargo build command
    Build {
        #[arg(long, short)]
        release: bool,
    },
    /// Create a new Picodata plugin
    New {
        #[arg(value_name = "path")]
        path: PathBuf,
        /// Disable the automatic git initialization
        #[arg(long)]
        without_git: bool,
        /// Initiate plugin as a subcrate of workspace
        #[arg(long)]
        workspace: bool,
    },
    /// Create a new Picodata plugin in an existing directory
    Init {
        /// Disable the automatic git initialization
        #[arg(long)]
        without_git: bool,
        /// Initiate plugin as a subcrate of workspace
        #[arg(long)]
        workspace: bool,
    },
}

#[derive(Subcommand)]
enum Config {
    /// Apply services config on Picodata cluster started by the Run command
    Apply {
        /// Path to config of the plugin
        #[arg(
            short,
            long,
            value_name = "CONFIG",
            default_value = "plugin_config.yaml"
        )]
        config_path: PathBuf,
        /// Path to data directory of the cluster
        #[arg(long, value_name = "DATA_DIR", default_value = "./tmp")]
        data_dir: PathBuf,
    },
}

/// Separated supervisor process to kill child processes if the parent is dead.
///
/// # Safety
///
/// This function is safe because it uses safety functions from `libc` without side effects.
fn child_killer(master_pid: u32) {
    unsafe {
        libc::setsid();
    }

    let master_pid = i32::try_from(master_pid).expect("Master PID to big");

    loop {
        let ret = unsafe { libc::kill(master_pid, 0) };
        if ret != 0 {
            unsafe { libc::killpg(master_pid, libc::SIGKILL) };
            break;
        }

        thread::sleep(Duration::from_secs(CK_CHECK_PARRENT_INTERVAL_SEC));
    }

    process::exit(0)
}

fn main() -> Result<()> {
    let self_pid = std::process::id();

    unsafe {
        match fork() {
            Ok(ForkResult::Parent { .. }) => (),
            Ok(ForkResult::Child) => child_killer(self_pid),
            Err(_) => log::warn!("Error run supervisor process"),
        }
    }

    colog::init();
    let cli = Cli::parse_from(env::args().skip(1));

    match cli.command {
        Command::Run {
            topology,
            data_dir,
            disable_install_plugins: disable_plugin_install,
            base_http_port,
            picodata_path,
            base_pg_port,
            release,
            target_dir,
        } => commands::run::cmd(
            &topology,
            &data_dir,
            disable_plugin_install,
            base_http_port,
            &picodata_path,
            base_pg_port,
            release,
            &target_dir,
        )
        .context("failed to execute Run command")?,
        Command::Stop { data_dir } => {
            commands::stop::cmd(&data_dir).context("failed to execute \"stop\" command")?;
        }
        Command::Clean { data_dir } => {
            commands::clean::cmd(&data_dir).context("failed to execute \"clean\" command")?;
        }
        Command::Plugin { command } => match command {
            Plugin::Pack { debug, target_dir } => {
                commands::plugin::pack::cmd(debug, &target_dir)
                    .context("failed to execute \"pack\" command")?;
            }
            Plugin::Build { release } => {
                commands::plugin::build::cmd(release)
                    .context("failed to execute \"build\" command")?;
            }
            Plugin::New {
                path,
                without_git,
                workspace,
            } => commands::plugin::new::cmd(Some(&path), without_git, workspace)
                .context("failed to execute \"plugin new\" command")?,
            Plugin::Init {
                without_git,
                workspace,
            } => commands::plugin::new::cmd(None, without_git, workspace)
                .context("failed to execute \"init\" command")?,
        },
        Command::Config { command } => match command {
            Config::Apply {
                config_path,
                data_dir,
            } => commands::config::apply::cmd(&config_path, &data_dir)
                .context("failed to execute \"config apply\" command")?,
        },
    };

    Ok(())
}
