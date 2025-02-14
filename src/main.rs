use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nix::unistd::{fork, ForkResult};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process, thread,
    time::Duration,
};

mod commands;

const CK_CHECK_PARRENT_INTERVAL_SEC: u64 = 3;

const CARING_FISH: &str = r#"
  ________________________________________
/ It seems to me, that you are trying to \
| run pike outside Plugin directory, try |
| using --plugin-dir flag or move into   |
\ plugin directory.                      /
 ----------------------------------------
                    |
                    |
                   ,|.
                  ,\|/.
                ,' .V. `.
               / .     . \
              /_`       '_\
             ,' .:     ;, `.
             |@)|  . .  |(@|
        ,-._ `._';  .  :`_,' _,-.
       '--  `-\ /,-===-.\ /-'  --`
      (----  _|  ||___||  |_  ----)
       `._,-'  \  `-.-'  /  `-._,'
                `-.___,-'
 "#;

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
        base_http_port: u16,
        /// Port for Pgproto server
        #[arg(long, default_value = "5432")]
        base_pg_port: u16,
        /// Specify path to picodata binary
        #[arg(long, value_name = "BINARY_PATH", default_value = "picodata")]
        picodata_path: PathBuf,
        /// Run release version of plugin
        #[arg(long)]
        release: bool,
        /// Change target folder
        #[arg(long, value_name = "TARGET_DIR", default_value = "target")]
        target_dir: PathBuf,
        /// Run cluster in background
        #[arg(long, short)]
        daemon: bool,
        /// Disable colors in stdout
        #[arg(long)]
        disable_colors: bool,
        /// Path to plugin folder
        #[arg(long, value_name = "PLUGIN_PATH", default_value = "./")]
        plugin_path: PathBuf,
    },
    /// Stop Picodata cluster
    Stop {
        #[arg(long, value_name = "DATA_DIR", default_value = "./tmp")]
        data_dir: PathBuf,
        /// Path to plugin folder
        #[arg(long, value_name = "PLUGIN_PATH", default_value = "./")]
        plugin_path: PathBuf,
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
        /// Path to plugin folder
        #[arg(long, value_name = "PLUGIN_PATH", default_value = "./")]
        plugin_path: PathBuf,
    },
    /// Alias for cargo build command
    Build {
        /// Change target folder
        #[arg(long, value_name = "TARGET_DIR", default_value = "target")]
        target_dir: PathBuf,
        /// Build release version of plugin
        #[arg(long, short)]
        release: bool,
        /// Path to plugin folder
        #[arg(long, value_name = "PLUGIN_PATH", default_value = "./")]
        plugin_path: PathBuf,
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
fn run_child_killer() {
    let master_pid = std::process::id();

    unsafe {
        match fork() {
            Ok(ForkResult::Parent { .. }) => return,
            Ok(ForkResult::Child) => (),
            Err(_) => log::warn!("Error run supervisor process"),
        }

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

fn check_plugin_directory() {
    if !Path::new("./topology.toml").exists() {
        println!("{}", CARING_FISH);

        process::exit(1);
    }
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
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
            daemon,
            disable_colors,
            plugin_path,
        } => {
            check_plugin_directory();

            if !daemon {
                run_child_killer();
            }
            let topology: commands::run::Topology = toml::from_str(
                &fs::read_to_string(plugin_path.join(&topology))
                    .context(format!("failed to read {}", &topology.display()))?,
            )
            .context(format!(
                "failed to parse .toml file of {}",
                topology.display()
            ))?;

            let mut params = commands::run::ParamsBuilder::default()
                .topology(topology)
                .data_dir(data_dir)
                .disable_plugin_install(disable_plugin_install)
                .base_http_port(base_http_port)
                .picodata_path(picodata_path)
                .base_pg_port(base_pg_port)
                .use_release(release)
                .target_dir(target_dir)
                .daemon(daemon)
                .disable_colors(disable_colors)
                .plugin_path(plugin_path)
                .build()
                .unwrap();
            commands::run::cmd(&mut params).context("failed to execute Run command")?;
        }
        Command::Stop {
            data_dir,
            plugin_path,
        } => {
            check_plugin_directory();

            run_child_killer();
            let params = commands::stop::ParamsBuilder::default()
                .data_dir(data_dir)
                .plugin_path(plugin_path)
                .build()
                .unwrap();
            commands::stop::cmd(&params).context("failed to execute \"stop\" command")?;
        }
        Command::Clean { data_dir } => {
            run_child_killer();
            commands::clean::cmd(&data_dir).context("failed to execute \"clean\" command")?;
        }
        Command::Plugin { command } => {
            run_child_killer();
            match command {
                Plugin::Pack {
                    debug,
                    target_dir,
                    plugin_path,
                } => {
                    check_plugin_directory();

                    commands::plugin::pack::cmd(debug, &target_dir, &plugin_path)
                        .context("failed to execute \"pack\" command")?;
                }
                Plugin::Build {
                    release,
                    target_dir,
                    plugin_path,
                } => {
                    check_plugin_directory();

                    commands::plugin::build::cmd(release, &target_dir, &plugin_path)
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
            }
        }
        Command::Config { command } => {
            run_child_killer();
            match command {
                Config::Apply {
                    config_path,
                    data_dir,
                } => {
                    let params = commands::config::apply::ParamsBuilder::default()
                        .config_path(config_path)
                        .data_dir(data_dir)
                        .build()
                        .unwrap();
                    commands::config::apply::cmd(&params)
                        .context("failed to execute \"config apply\" command")?;
                }
            }
        }
    };

    Ok(())
}
