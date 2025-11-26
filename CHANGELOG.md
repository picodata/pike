# Change Log

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/) and this project adheres to [Semantic Versioning](http://semver.org/).

## [UNRELEASED]

### Added

- Support running SQL scripts before plugin installation
- Support running external plugins without a parent plugin project
- Add `--archive-name` option to `plugin pack`
- Add validation of picodata.yaml config
- Add `--picodata-path` option to `config apply` command
- Add optional `--no-build` flag to `plugin pack` command

### Changed

- Bump Picodata version to 25.4.2 in plugin template
- Bump Picotest version to 1.8.1 in plugin template

## [2.9.0]

### Added

- Add `--with-web-auth` flag to pike run, explicitly enabling WebUI authentication
- Add plugin lifecycle smoke test in plugin template
- Archive naming now includes OS identifier and variant: `<name>_<version>-<osid>_<variant>.tar.gz`
- OS detection for `plugin pack`:
  - Linux: parsing `/etc/os-release`
  - macOS: `sw_vers`
  - Rolling distro handling

### Changed
- Previous archive name `<name>-<version>.tar.gz` replaced by new format including OS suffix

### Notes
- If `VERSION_ID` is missing for a non‑rolling distro, the variant becomes `unknown`
- For known rolling distros without `VERSION_ID` the variant becomes `rolling`

## [2.8.0]

### Added

- use env variables for ipv4 addresses if they are set and cli arguments are not provided
- honorable mention in AUTHORS

## [2.7.1]

## Fixed
- waiting for leader id to be negotiated before enabling plugins
- waiting for node Online state before enabling plugins
- with the help of added checks for leader id and node online status, cluster no longer dies with "RAFT proposal dropped" error

## [2.7.0]

### Added

- Support running/stopping specific instance

### Fixed

- fix CI trigger

## [2.6.0]

### Added

- Add ability to set external path for plugins
- support stopping of specific cluster instance

## [2.5.0]

### Added

- Add ability to pass config in run command
- Add start alias for run command

### Changed

- Remove traceback print on run failure
- Better logs for clean and stop
- Check if cluster is already up during run command
- Improve error message for clean if there is no data dir
- Consider plugin name from Cargo.toml while packing plugin

### Fixed

- Fix log output in apply config command
- Add feature signal for nix
- Repair log output in apply config command

## [2.4.5]

### Changed

- Bump Picodata version from 25.1.2 to 25.2.1

## [2.4.4]

### Changed

- Bump Picodata version from 25.1.1 to 25.1.2 (665bfd2)

## [2.4.3]

### Changed

- Add rpc handle test to template (15e353b)
- Remove Option from Service config in template (841f0e9)
- Rename main service to example_service (5e98601)

### Fixed

- Add human readable error if Picodata not found (709ba2d)

## [2.4.2]

### Fixed

- Pid and kill not found in nix package (b1307e8)

## [2.4.1]

### Changed

- Improve plugin template (add rpc, http endpoints) (9379788, 655dd63)

### Fixed

- Add handling for picodata admin failure (54ddee2)

## [2.4.0]

### Added

- Method to return all properties of PicodataInstance (49629da)

## [2.3.1]

### Fixed

- Changed visibility of method for receiving pg port (98ad2e0)

## [2.3.0]

### Added

- Access to pg_port of picodata test instance (9062690)
- Implement `enter` command (7908f36)
- Add `--plugin-path` flag for clean command (bca646d)
- Add `--no-build` flag to pike run (4a730d4)
- Add warnings for alien field in `topology.toml` (1f42af5)

### Fixed

- Fix apply config for service with dash in name (16b674c)

## [2.2.2]

### Changed

- Rename plugin tarball (6296765)

## [2.2.1]

### Fixed

- Run build if assets or migrations were changed (a5d4b8ab)

## [2.2.0]

### Added

- Support adding custom assets under desired names (921dec0)

### Changed

- Bind http port on 0.0.0.0 (4b92057a)

### Fixed

- Move all necessary files to the root folder of the workspace (fa293fc9)

## [2.1.2]

### Fixed

- Invert version check. This change gives us ability to run pike with picodata version > 25.1 (3b6b608)

## [2.1.1]

### Changed

- Get names of instances from cluster (8fcf671a)
- Accelerated cluster launch (8fcf671a)
- Move built files in archive into plugin_name/version subfolder (0d96cea0)

### Fixed

- Pass replication_factor to cluster config (d32c364c)

## [2.1.0]

### Added

- Support passing plugin config as a map to the "config apply" command API (a2a1a8b)

### Fixed

- Sort migration files, before inserting into `manifest.yaml` (fc01dca)

## [2.0.2]

### Changed

- Remove required minimal rust version for Pike (7a7d730)
- Update pike version in template (aa2d3cf)
- Remove unused dependencies from template (aa2d3cf)

## [2.0.1]

### Changed

- Change pike dependency source in template (b33fc168)

## [2.0.0]

### Breaking Changes

- Move to `25.1.1` Picodata version, rename `config.yaml` to `picodata.yaml` (90468b15)
- Plugin pack command now saves the plugin archive in `release/debug` folder (36b20b3f)
- Change `topology.toml` format: `tiers` renamed to `tier`, `instances` renamed to `replicasets`. Add new section `plugin`. (678f1c17)

### Added

- Implement `plugin add` command for workspaces (789c9664)
- Support working with multiple plugins and custom assets (98f7ac8e)
- Expose `PicodataInstance` object (3bd69626)
- Run Pike without a plugin directory (2138e00b)
- Add hints when running Pike in the wrong folder (d7785a13)
- Pass topology as a structure in library function (f9478c33)
- Add `--plugin-path` parameter to `run/stop/pack/build` commands (403ae68a)

### Changed

- Set the latest version for Cargo resolver in template (09cde0a0)
- Clean plugin folder from trash in workspaces (67ed7f79)
- Update Rust version (568b75c6)
- Improve `run` command behavior:
    - Add daemon mode (0cd689e9)
    - Improve logs (d07baf58)
    - Write logs to files per instance (d07baf58)
    - Add colored instance name prefix in stdout logs (d07baf58)
- Improve `Ctrl+C` handling for proper shutdown (701be745)
- Enhance error handling during instance stop (d233a74d)
- Forward output from `picodata admin` in `config apply` command (05bae132)

### Fixed

- Adjust `config apply` for workspaces (a22a7adf)
- Fix `picodata.yaml` copying to workspace root (d7b7edb3)
- Fix query for migration variables apply (e07fc8da)
- Fix handling of bad args check in `config apply` tests (90a0818d)
- Fix `--target-dir` flag behavior in `pack` command (7691788a)

## [1.0.0]

This is the first public release of the project.
