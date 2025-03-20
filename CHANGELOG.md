# Change Log

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/) and this project adheres to [Semantic Versioning](http://semver.org/).

## [UNRELEASED]

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
