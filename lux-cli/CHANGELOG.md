# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.39.2](https://github.com/lumen-oss/lux/compare/v0.39.1...v0.39.2) `lux-cli` - 2026-07-22

### Added
- *(ui)* slow down moon spinner animation ([#1731](https://github.com/lumen-oss/lux/pull/1731))

## [0.39.1](https://github.com/lumen-oss/lux/compare/v0.39.0...v0.39.1) `lux-cli` - 2026-07-21

### Added
- *(tracing)* more trace spans ([#1718](https://github.com/lumen-oss/lux/pull/1718))

## [0.39.0](https://github.com/lumen-oss/lux/compare/v0.38.1...v0.39.0) `lux-cli` - 2026-07-20

### Added
- *(tracing)* add cli tracing output
- reduce emojis in progress spinners
- more output on `--verbose`
- instrument build backends + async tasks
- format span fields in progress spinners if present
- output with debug level if `--verbose` is set
- *(tracing)* add fields to spans
- [**breaking**] replace progress bars with `tracing` calls [WIP]

### Fixed
- don't display spinners if not in a terminal/tty

### Other
- *(tracing)* make CI happy
- re-add `lx unpack` implementation with tracing info messages

## [0.38.1](https://github.com/lumen-oss/lux/compare/v0.38.0...v0.38.1) `lux-cli` - 2026-07-16

### Other
- updated the following local packages: lux-lib

## [0.37.1](https://github.com/lumen-oss/lux/compare/v0.37.0...v0.37.1) `lux-cli` - 2026-07-15

### Fixed
- *(cli)* prevent miette from intercepting clap help output ([#1690](https://github.com/lumen-oss/lux/pull/1690))

## [0.37.0](https://github.com/lumen-oss/lux/compare/v0.36.1...v0.37.0) `lux-cli` - 2026-07-14

### Added
- [**breaking**] improved error messages

### Fixed
- *(new)* don't prompt to overwrite project if none exists

## [0.36.1](https://github.com/lumen-oss/lux/compare/v0.36.0...v0.36.1) `lux-cli` - 2026-07-13

### Dependencies
- *(deps)* bump emmylua_formatter from 0.23.2 to 0.24.0 ([#1677](https://github.com/lumen-oss/lux/pull/1677))

## [0.36.0](https://github.com/lumen-oss/lux/compare/v0.35.3...v0.36.0) `lux-cli` - 2026-07-12

### Added
- [**breaking**] replace `--porcelain` with `--output-format` ([#1674](https://github.com/lumen-oss/lux/pull/1674))
- *(check)* support specifying directories ([#1663](https://github.com/lumen-oss/lux/pull/1663))
- *(fmt)* [**breaking**] use `--path` flag

### Fixed
- *(lint)* [**breaking**] resolve CLI argument parsing regression in lx lint ([#1669](https://github.com/lumen-oss/lux/pull/1669))
- *(lua-api)*  fix broken `UserData` instances ([#1672](https://github.com/lumen-oss/lux/pull/1672))

## [0.35.3](https://github.com/lumen-oss/lux/compare/v0.35.2...v0.35.3) `lux-cli` - 2026-07-08

### Added
- *(dist)* single binary project distribution ([#1652](https://github.com/lumen-oss/lux/pull/1652))

### Dependencies
- *(deps)* bump octocrab from 0.53.1 to 0.54.0 ([#1660](https://github.com/lumen-oss/lux/pull/1660))

## [0.35.2](https://github.com/lumen-oss/lux/compare/v0.35.1...v0.35.2) `lux-cli` - 2026-07-06

### Added
- *(lint)* support specifying files & directories ([#1656](https://github.com/lumen-oss/lux/pull/1656))

## [0.35.1](https://github.com/lumen-oss/lux/compare/v0.35.0...v0.35.1) `lux-cli` - 2026-07-02

### Added
- *(debug)* add `debug toolchains` subcommand ([#1645](https://github.com/lumen-oss/lux/pull/1645))

## [0.35.0](https://github.com/lumen-oss/lux/compare/v0.34.2...v0.35.0) `lux-cli` - 2026-07-02

### Added
- *(upload)* support generating TOTP codes in CI ([#1643](https://github.com/lumen-oss/lux/pull/1643))
- *(upload)* [**breaking**] support two-factor authentication ([#1642](https://github.com/lumen-oss/lux/pull/1642))

## [0.34.1](https://github.com/lumen-oss/lux/compare/v0.34.0...v0.34.1) `lux-cli` - 2026-06-30

### Other
- update Cargo.lock dependencies

## [0.34.0](https://github.com/lumen-oss/lux/compare/v0.33.8...v0.34.0) `lux-cli` - 2026-06-29

### Added
- *(config)* add `luarc_file_name` option ([#1626](https://github.com/lumen-oss/lux/pull/1626))

## [0.33.8](https://github.com/lumen-oss/lux/compare/v0.33.7...v0.33.8) `lux-cli` - 2026-06-27

### Other
- roll back `http` dependency to 1.4.0.
  Note: 1.4.2 causes a test failure.

## [0.33.7](https://github.com/lumen-oss/lux/compare/v0.33.6...v0.33.7) `lux-cli` - 2026-06-26

### Other
- update Cargo.lock dependencies

## [0.33.6](https://github.com/lumen-oss/lux/compare/v0.33.5...v0.33.6) `lux-cli` - 2026-06-26

### Other
- update Cargo.lock dependencies

## [0.33.5](https://github.com/lumen-oss/lux/compare/v0.33.4...v0.33.5) `lux-cli` - 2026-06-26

### Added
- *(lux-lua)* distribute Lua type definitions ([#1522](https://github.com/lumen-oss/lux/pull/1522))

## [0.33.4](https://github.com/lumen-oss/lux/compare/v0.33.3...v0.33.4) `lux-cli` - 2026-06-24

### Fixed
- *(cli/new)* properly parse precise SPDX license IDs ([#1614](https://github.com/lumen-oss/lux/pull/1614))

## [0.33.3](https://github.com/lumen-oss/lux/compare/v0.33.2...v0.33.3) `lux-cli` - 2026-06-23

### Added
- *(fmt)* add support for specifying directories and files ([#1609](https://github.com/lumen-oss/lux/pull/1609))

## [0.33.2](https://github.com/lumen-oss/lux/compare/v0.33.1...v0.33.2) `lux-cli` - 2026-06-22

### Other
- update Cargo.lock dependencies

## [0.33.1](https://github.com/lumen-oss/lux/compare/v0.33.0...v0.33.1) `lux-cli` - 2026-06-17

### Added
- use `color_eyre` for colorful, consistent, and well formatted error reports ([#1601](https://github.com/lumen-oss/lux/pull/1601))

## [0.33.0](https://github.com/lumen-oss/lux/compare/v0.32.0...v0.33.0) `lux-cli` - 2026-06-16

### Fixed
- *(dependencies)* [**breaking**] improve git remote URL parsing ([#1595](https://github.com/lumen-oss/lux/pull/1595))

## [0.32.0](https://github.com/lumen-oss/lux/compare/v0.31.1...v0.32.0) `lux-cli` - 2026-06-15

### Added
- *(dist)* `lx dist flat-archive`
- `lx dist` skeleton
- config option to disable bin script wrapping ([#1586](https://github.com/lumen-oss/lux/pull/1586))

## [0.31.1](https://github.com/lumen-oss/lux/compare/v0.31.0...v0.31.1) `lux-cli` - 2026-06-05

### Other
- updated the following local packages: lux-lib

## [0.31.0](https://github.com/lumen-oss/lux/compare/v0.30.6...v0.31.0) `lux-cli` - 2026-06-05

### Dependencies
- *(deps)* bump octocrab from 0.51.0 to 0.53.0 ([#1564](https://github.com/lumen-oss/lux/pull/1564))

### Fixed
- *(fmt)* [**breaking**] allow workspace which resides in a different directory ([#1574](https://github.com/lumen-oss/lux/pull/1574))

## [0.30.6](https://github.com/lumen-oss/lux/compare/v0.30.5...v0.30.6) `lux-cli` - 2026-06-02

### Added
- *(generate-rockspec)* add `--porcelain` flag ([#1559](https://github.com/lumen-oss/lux/pull/1559))

## [0.30.5](https://github.com/lumen-oss/lux/compare/v0.30.4...v0.30.5) `lux-cli` - 2026-06-02

### Added
- *(pack)* support packing individual workspace members ([#1554](https://github.com/lumen-oss/lux/pull/1554))

### Dependencies
- *(deps)* bump serial_test from 3.4.0 to 3.5.0 ([#1556](https://github.com/lumen-oss/lux/pull/1556))

## [0.30.4](https://github.com/lumen-oss/lux/compare/v0.30.3...v0.30.4) `lux-cli` - 2026-05-31

### Other
- updated the following local packages: lux-lib

## [0.30.3](https://github.com/lumen-oss/lux/compare/v0.30.2...v0.30.3) `lux-cli` - 2026-05-28

### Other
- update Cargo.lock dependencies

## [0.30.2](https://github.com/lumen-oss/lux/compare/v0.30.1...v0.30.2) `lux-cli` - 2026-05-27

### Other
- updated the following local packages: lux-lib

## [0.30.1](https://github.com/lumen-oss/lux/compare/v0.30.0...v0.30.1) `lux-cli` - 2026-05-26

### Fixed
- *(cli/run)* ensure unique `--package` argument ([#1527](https://github.com/lumen-oss/lux/pull/1527))

## [0.30.0](https://github.com/lumen-oss/lux/compare/v0.29.3...v0.30.0) `lux-cli` - 2026-05-26

### Added
- support local dependencies ([#1508](https://github.com/lumen-oss/lux/pull/1508))
- support workspaces with multiple projects ([#1503](https://github.com/lumen-oss/lux/pull/1503))

### Dependencies
- *(deps)* bump git2 from 0.20.4 to 0.21.0 ([#1516](https://github.com/lumen-oss/lux/pull/1516))

### Other
- update flake.lock ([#1521](https://github.com/lumen-oss/lux/pull/1521))

## [0.29.3](https://github.com/lumen-oss/lux/compare/v0.29.2...v0.29.3) `lux-cli` - 2026-05-19

### Other
- updated the following local packages: lux-lib

## [0.29.2](https://github.com/lumen-oss/lux/compare/v0.29.1...v0.29.2) `lux-cli` - 2026-05-18

### Dependencies
- *(deps)* bump octocrab from 0.50.0 to 0.51.0 ([#1511](https://github.com/lumen-oss/lux/pull/1511))

## [0.29.1](https://github.com/lumen-oss/lux/compare/v0.29.0...v0.29.1) `lux-cli` - 2026-05-14

### Added
- *(format)* luafmt backend ([#1505](https://github.com/lumen-oss/lux/pull/1505))

## [0.29.0](https://github.com/lumen-oss/lux/compare/v0.28.9...v0.29.0) `lux-cli` - 2026-05-09

### Added
- [**breaking**] set user agent header for web requests ([#1481](https://github.com/lumen-oss/lux/pull/1481))

## [0.28.9](https://github.com/lumen-oss/lux/compare/v0.28.8...v0.28.9) `lux-cli` - 2026-05-06

### Other
- update Cargo.lock dependencies

## [0.28.8](https://github.com/lumen-oss/lux/compare/v0.28.7...v0.28.8) `lux-cli` - 2026-05-05

### Dependencies
- *(deps)* bump octocrab from 0.49.7 to 0.50.0 ([#1486](https://github.com/lumen-oss/lux/pull/1486))

## [0.28.7](https://github.com/lumen-oss/lux/compare/v0.28.6...v0.28.7) `lux-cli` - 2026-05-04

### Added
- *(fmt)* source editorconfig for emmylua-codestyle ([#1483](https://github.com/lumen-oss/lux/pull/1483))

## [0.28.6](https://github.com/lumen-oss/lux/compare/v0.28.5...v0.28.6) `lux-cli` - 2026-04-26

### Added
- *(fmt)* source editorconfig for stylua ([#1472](https://github.com/lumen-oss/lux/pull/1472))

### Fixed
- *(fmt)* allow specifying relative paths ([#1473](https://github.com/lumen-oss/lux/pull/1473))

## [0.28.5](https://github.com/lumen-oss/lux/compare/v0.28.4...v0.28.5) `lux-cli` - 2026-04-25

### Other
- update Cargo.toml dependencies

## [0.28.4](https://github.com/lumen-oss/lux/compare/v0.28.3...v0.28.4) `lux-cli` - 2026-04-24

### Other
- update Cargo.lock dependencies

## [0.28.3](https://github.com/lumen-oss/lux/compare/v0.28.2...v0.28.3) `lux-cli` - 2026-04-23

### Other
- update Cargo.lock dependencies

## [0.28.2](https://github.com/lumen-oss/lux/compare/v0.28.1...v0.28.2) `lux-cli` - 2026-04-15

### Dependencies
- *(deps)* bulk update ([#1446](https://github.com/lumen-oss/lux/pull/1446))

## `lux-cli` - [0.28.1](https://github.com/lumen-oss/lux/compare/v0.28.0...v0.28.1) - 2026-04-14

### Added
- `no_prompt` option ([#1441](https://github.com/lumen-oss/lux/pull/1441))

## `lux-cli` - [0.28.0](https://github.com/lumen-oss/lux/compare/v0.27.0...v0.28.0) - 2026-04-08

### Added
- [**breaking**] remove From<bool> for BuildBehaviour ([#1425](https://github.com/lumen-oss/lux/pull/1425))

## `lux-cli` - [0.26.4](https://github.com/lumen-oss/lux/compare/v0.26.3...v0.26.4) - 2026-03-24

### Added
- add `lx sync` command

## `lux-cli` - [0.26.3](https://github.com/lumen-oss/lux/compare/v0.26.2...v0.26.3) - 2026-03-20

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.26.2](https://github.com/lumen-oss/lux/compare/v0.26.1...v0.26.2) - 2026-03-19

### Dependencies
- *(deps)* bump clap from 4.5.60 to 4.6.0 ([#1386](https://github.com/lumen-oss/lux/pull/1386))
- *(deps)* bump clap_complete from 4.5.61 to 4.6.0 ([#1384](https://github.com/lumen-oss/lux/pull/1384))

## `lux-cli` - [0.26.1](https://github.com/lumen-oss/lux/compare/v0.26.0...v0.26.1) - 2026-03-12

### Fixed
- run `harper-cli` on README and fix mistakes

## `lux-cli` - [0.26.0](https://github.com/lumen-oss/lux/compare/v0.25.3...v0.26.0) - 2026-03-12

### Added
- *(ui)* enable OSC native terminal progress bar support ([#1369](https://github.com/lumen-oss/lux/pull/1369))

### Dependencies
- *(deps)* bump spinners from 4.1.1 to 4.2.0 ([#1368](https://github.com/lumen-oss/lux/pull/1368))

## `lux-cli` - [0.26.0](https://github.com/lumen-oss/lux/compare/v0.25.3...v0.26.0) - 2026-03-12

### Added
- *(ui)* enable OSC native terminal progress bar support ([#1369](https://github.com/lumen-oss/lux/pull/1369))

### Dependencies
- *(deps)* bump spinners from 4.1.1 to 4.2.0 ([#1368](https://github.com/lumen-oss/lux/pull/1368))

## `lux-cli` - [0.25.3](https://github.com/lumen-oss/lux/compare/v0.25.2...v0.25.3) - 2026-02-23

### Dependencies
- *(deps)* bump serial_test from 3.3.1 to 3.4.0 ([#1358](https://github.com/lumen-oss/lux/pull/1358))
- *(deps)* bump termtree from 0.5.1 to 1.0.0 ([#1346](https://github.com/lumen-oss/lux/pull/1346))

## `lux-cli` - [0.25.2](https://github.com/lumen-oss/lux/compare/v0.25.1...v0.25.2) - 2026-02-09

### Added
- (unofficial) Android compilation target support ([#1338](https://github.com/lumen-oss/lux/pull/1338))

### Other
- *(readme)* update package badge ([#1336](https://github.com/lumen-oss/lux/pull/1336))
- *(readme)* add missing Lua 5.5 reference
- *(deps)* bump emmylua_check to 0.20.0 ([#1330](https://github.com/lumen-oss/lux/pull/1330))

## `lux-cli` - [0.25.1](https://github.com/lumen-oss/lux/compare/v0.25.0...v0.25.1) - 2026-01-30

### Fixed
- *(tests)* install vendored project to temp directory

### Other
- *(readme)* add xtask snippet for Lua 5.5

## `lux-cli` - [0.25.0](https://github.com/lumen-oss/lux/compare/v0.24.2...v0.25.0) - 2026-01-29

### Added
- *(fmt)* [**breaking**] format `test` and `tests` directories
- [**breaking**] support lua 5.5 ([#1258](https://github.com/lumen-oss/lux/pull/1258))
- *(fmt)* [**breaking**] format `spec` directory ([#1318](https://github.com/lumen-oss/lux/pull/1318))
- *(lux-cli)* vendor with luau for sandboxing ([#1309](https://github.com/lumen-oss/lux/pull/1309))

### Other
- *(deps)* bump whoami from 2.0.0 to 2.1.0 ([#1320](https://github.com/lumen-oss/lux/pull/1320))
- *(readme)* update luacheck url ([#1319](https://github.com/lumen-oss/lux/pull/1319))

## `lux-cli` - [0.24.2](https://github.com/lumen-oss/lux/releases/tag/v0.24.2) - 2026-01-21

### Fixed
- *(test/busted-nlua)* unbreak on macOS ([#1304](https://github.com/lumen-oss/lux/pull/1304))

### Other
- release ([#1306](https://github.com/lumen-oss/lux/pull/1306))

## `lux-cli` - [0.24.2](https://github.com/lumen-oss/lux/compare/v0.24.1...v0.24.2) - 2026-01-21

### Fixed
- *(test/busted-nlua)* unbreak on macOS ([#1304](https://github.com/lumen-oss/lux/pull/1304))

## `lux-cli` - [0.24.1](https://github.com/lumen-oss/lux/releases/tag/v0.24.1) - 2026-01-21

### Added
- *(fmt)* include file path in error message ([#1302](https://github.com/lumen-oss/lux/pull/1302))

### Other
- release ([#1301](https://github.com/lumen-oss/lux/pull/1301))

## `lux-cli` - [0.24.1](https://github.com/lumen-oss/lux/compare/v0.24.0...v0.24.1) - 2026-01-21

### Added
- *(fmt)* include file path in error message ([#1302](https://github.com/lumen-oss/lux/pull/1302))

## `lux-cli` - [0.24.0](https://github.com/lumen-oss/lux/compare/v0.23.1...v0.24.0) - 2026-01-17

### Added
- [**breaking**] option to build with vendored directory ([#1283](https://github.com/lumen-oss/lux/pull/1283))
- `vendor` command

## `lux-cli` - [0.23.1](https://github.com/lumen-oss/lux/compare/v0.23.0...v0.23.1) - 2026-01-17

### Fixed
- *(install-rockspec)* dependencies not installed ([#1292](https://github.com/lumen-oss/lux/pull/1292))

## `lux-cli` - [0.23.0](https://github.com/lumen-oss/lux/compare/v0.22.5...v0.23.0) - 2026-01-15

### Fixed
- *(lint)* [**breaking**] don't install luacheck to project tree ([#1288](https://github.com/lumen-oss/lux/pull/1288))

## `lux-cli` - [0.22.5](https://github.com/lumen-oss/lux/compare/v0.22.4...v0.22.5) - 2026-01-13

### Fixed
- *(lux-cli)* make `lx config edit` work with `VISUAL="nvim --cmd 'let g:flatten_wait=1'"` ([#1280](https://github.com/lumen-oss/lux/pull/1280))

## `lux-cli` - [0.22.4](https://github.com/lumen-oss/lux/compare/v0.22.3...v0.22.4) - 2026-01-12

### Other
- *(deps)* bump whoami from 1.6.1 to 2.0.0 ([#1275](https://github.com/lumen-oss/lux/pull/1275))
- *(deps)* bump serial_test from 3.2.0 to 3.3.1 ([#1272](https://github.com/lumen-oss/lux/pull/1272))
- *(deps)* bump emmylua_codestyle from 0.5.0 to 0.6.0 ([#1263](https://github.com/lumen-oss/lux/pull/1263))

## `lux-cli` - [0.22.3](https://github.com/lumen-oss/lux/compare/v0.22.2...v0.22.3) - 2025-12-24

### Other
- *(deps)* bump octocrab from 0.48.1 to 0.49.2 ([#1256](https://github.com/lumen-oss/lux/pull/1256))

## `lux-cli` - [0.22.2](https://github.com/lumen-oss/lux/compare/v0.22.1...v0.22.2) - 2025-12-12

### Other
- update Cargo.lock dependencies

## `lux-cli` - [0.22.1](https://github.com/lumen-oss/lux/compare/v0.22.0...v0.22.1) - 2025-12-12

### Other
- update Cargo.lock dependencies

## `lux-cli` - [0.22.0](https://github.com/lumen-oss/lux/compare/v0.21.0...v0.22.0) - 2025-12-06

### Fixed
- *(exec)* [**breaking**] always build project first ([#1236](https://github.com/lumen-oss/lux/pull/1236))

## `lux-cli` - [0.21.0](https://github.com/lumen-oss/lux/compare/v0.20.4...v0.21.0) - 2025-12-03

### Added
- dependencies update + better error messages when failing to parse rockspec ([#1202](https://github.com/lumen-oss/lux/pull/1202))

### Other
- *(deps)* bump spdx from 0.12.0 to 0.13.0 ([#1226](https://github.com/lumen-oss/lux/pull/1226))
- [**breaking**] disallow panic, expect and unwrap ([#1223](https://github.com/lumen-oss/lux/pull/1223))

## `lux-cli` - [0.20.4](https://github.com/lumen-oss/lux/compare/v0.20.3...v0.20.4) - 2025-12-01

### Other
- release ([#1220](https://github.com/lumen-oss/lux/pull/1220))

## `lux-cli` - [0.20.4](https://github.com/lumen-oss/lux/compare/v0.20.3...v0.20.4) - 2025-12-01

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.20.3](https://github.com/lumen-oss/lux/compare/v0.20.2...v0.20.3) - 2025-11-30

### Other
- release ([#1212](https://github.com/lumen-oss/lux/pull/1212))

## `lux-cli` - [0.20.3](https://github.com/lumen-oss/lux/compare/v0.20.2...v0.20.3) - 2025-11-30

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.20.2](https://github.com/lumen-oss/lux/compare/v0.20.1...v0.20.2) - 2025-11-30

### Other
- fix clippy warnings

## `lux-cli` - [0.20.1](https://github.com/lumen-oss/lux/compare/v0.20.0...v0.20.1) - 2025-11-10

### Added
- improve lux loader warning for `lx path` ([#1195](https://github.com/lumen-oss/lux/pull/1195))

## `lux-cli` - [0.20.0](https://github.com/lumen-oss/lux/compare/v0.19.0...v0.20.0) - 2025-11-05

### Added
- *(git)* add ssh auth callback ([#1152](https://github.com/lumen-oss/lux/pull/1152))

### Fixed
- *(cli/fmt)* [**breaking**] `--backend <BACKEND>` flag instead of arg ([#1182](https://github.com/lumen-oss/lux/pull/1182))

## `lux-cli` - [0.19.0](https://github.com/lumen-oss/lux/compare/v0.18.11...v0.19.0) - 2025-11-04

### Other
- *(uninstall)* [**breaking**] use `bon::Builder`

## `lux-cli` - [0.18.11](https://github.com/lumen-oss/lux/compare/v0.18.10...v0.18.11) - 2025-11-03

### Fixed
- *(test)* correct test executable in error message
- *(test)* outdated non-project root error message

### Other
- release ([#1171](https://github.com/lumen-oss/lux/pull/1171))

## `lux-cli` - [0.18.11](https://github.com/lumen-oss/lux/compare/v0.18.10...v0.18.11) - 2025-11-03

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.18.10](https://github.com/lumen-oss/lux/compare/v0.18.9...v0.18.10) - 2025-11-02

### Fixed
- *(cli/shell)* install Lua version if missing
- *(cli/lua)* install Lua version if missing

## `lux-cli` - [0.18.9](https://github.com/lumen-oss/lux/compare/v0.18.8...v0.18.9) - 2025-11-02

### Other
- release ([#1161](https://github.com/lumen-oss/lux/pull/1161))

## `lux-cli` - [0.18.9](https://github.com/lumen-oss/lux/compare/v0.18.8...v0.18.9) - 2025-11-02

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.18.8](https://github.com/lumen-oss/lux/compare/v0.18.7...v0.18.8) - 2025-10-29

### Added
- *(cli)* infer --lua-version 5.1 from --nvim ([#1151](https://github.com/lumen-oss/lux/pull/1151))

## `lux-cli` - [0.18.7](https://github.com/lumen-oss/lux/compare/v0.18.6...v0.18.7) - 2025-10-21

### Other
- *(deps)* bump emmylua_check from 0.15.0 to 0.16.0 ([#1142](https://github.com/lumen-oss/lux/pull/1142))

## `lux-cli` - [0.18.6](https://github.com/lumen-oss/lux/compare/v0.18.5...v0.18.6) - 2025-10-19

### Fixed
- *(lint)* always disable lux loader ([#1136](https://github.com/lumen-oss/lux/pull/1136))

## `lux-cli` - [0.18.5](https://github.com/lumen-oss/lux/compare/v0.18.4...v0.18.5) - 2025-10-16

### Other
- release ([#1128](https://github.com/lumen-oss/lux/pull/1128))

## `lux-cli` - [0.18.5](https://github.com/lumen-oss/lux/compare/v0.18.4...v0.18.5) - 2025-10-16

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.18.4](https://github.com/lumen-oss/lux/compare/v0.18.3...v0.18.4) - 2025-10-15

### Fixed
- *(windows)* unset readonly attribute before cleaning .git directory ([#1124](https://github.com/lumen-oss/lux/pull/1124))

## `lux-cli` - [0.18.3](https://github.com/lumen-oss/lux/compare/v0.18.2...v0.18.3) - 2025-10-15

### Other
- *(deps)* bump emmylua_check from 0.14.0 to 0.15.0 ([#1117](https://github.com/lumen-oss/lux/pull/1117))

## `lux-cli` - [0.18.2](https://github.com/lumen-oss/lux/compare/v0.18.1...v0.18.2) - 2025-10-12

### Other
- *(deps)* bump octocrab from 0.46.0 to 0.47.0 ([#1109](https://github.com/lumen-oss/lux/pull/1109))
- *(deps)* bump octocrab from 0.45.0 to 0.46.0 ([#1096](https://github.com/lumen-oss/lux/pull/1096))
- update README to reflect .luarc generation

## `lux-cli` - [0.18.1](https://github.com/lumen-oss/lux/compare/v0.18.0...v0.18.1) - 2025-09-24

### Other
- update homepage and PKGBUILD conflicts
- *(deps)* bump emmylua_check from 0.13.0 to 0.14.0 ([#1082](https://github.com/lumen-oss/lux/pull/1082))
- *(readme)* add note about statically linking gpgme

## `lux-cli` - [0.18.0](https://github.com/lumen-oss/lux/compare/v0.17.1...v0.18.0) - 2025-09-17

### Added
- [**breaking**] don't expose `git-url-parse` types ([#1073](https://github.com/lumen-oss/lux/pull/1073))

### Other
- *(deps)* replace unmaintained `tempdir` with `tempfile` ([#1074](https://github.com/lumen-oss/lux/pull/1074))
- *(deps)* bump inquire from 0.8.0 to 0.9.1 ([#1069](https://github.com/lumen-oss/lux/pull/1069))

## `lux-cli` - [0.17.1](https://github.com/lumen-oss/lux/compare/v0.17.0...v0.17.1) - 2025-09-15

### Other
- *(deps)* bulk update ([#1066](https://github.com/lumen-oss/lux/pull/1066))
- *(docs)* rename nvim-neorocks -> lumen-oss ([#1057](https://github.com/lumen-oss/lux/pull/1057))
- *(deps)* bump emmylua_check from 0.12.0 to 0.13.0 ([#1055](https://github.com/lumen-oss/lux/pull/1055))
- *(readme)* exclude unsupported repos in packaging status badge

## `lux-cli` - [0.15.1](https://github.com/lumen-oss/lux/compare/v0.15.0...v0.15.1) - 2025-08-16

### Other
- release ([#982](https://github.com/lumen-oss/lux/pull/982))

## `lux-cli` - [0.15.1](https://github.com/lumen-oss/lux/compare/v0.15.0...v0.15.1) - 2025-08-16

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.15.0](https://github.com/lumen-oss/lux/compare/v0.14.0...v0.15.0) - 2025-08-14

### Other
- *(operations)* [**breaking**] rename `Remove` to `Uninstall` for consistent terminology ([#795](https://github.com/lumen-oss/lux/pull/795))

## `lux-cli` - [0.14.0](https://github.com/lumen-oss/lux/compare/v0.13.3...v0.14.0) - 2025-08-14

### Other
- [**breaking**] binary package distributions ([#877](https://github.com/lumen-oss/lux/pull/877))

## `lux-cli` - [0.13.3](https://github.com/lumen-oss/lux/compare/v0.13.2...v0.13.3) - 2025-08-13

### Other
- *(deps)* bump spdx from 0.10.8 to 0.11.0 ([#969](https://github.com/lumen-oss/lux/pull/969))
- *(deps)* bump emmylua_check from 0.10.0 to 0.11.0 ([#970](https://github.com/lumen-oss/lux/pull/970))

## `lux-cli` - [0.13.2](https://github.com/lumen-oss/lux/compare/v0.13.1...v0.13.2) - 2025-08-08

### Other
- release ([#962](https://github.com/lumen-oss/lux/pull/962))

## `lux-cli` - [0.13.2](https://github.com/lumen-oss/lux/compare/v0.13.1...v0.13.2) - 2025-08-08

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.13.1](https://github.com/lumen-oss/lux/compare/v0.13.0...v0.13.1) - 2025-08-05

### Added
- `lx check` command for luaCATS typechecks ([#849](https://github.com/lumen-oss/lux/pull/849))

## `lux-cli` - [0.13.0](https://github.com/lumen-oss/lux/compare/v0.12.1...v0.13.0) - 2025-08-03

### Added
- *(build)* [**breaking**] pass in `LuaInstallation`
- [**breaking**] manage lua installations internally

### Other
- *(build)* [**breaking**] tidy up builder

## `lux-cli` - [0.12.1](https://github.com/lumen-oss/lux/compare/v0.12.0...v0.12.1) - 2025-07-31

### Other
- release ([#943](https://github.com/lumen-oss/lux/pull/943))

## `lux-cli` - [0.12.1](https://github.com/lumen-oss/lux/compare/v0.12.0...v0.12.1) - 2025-07-31

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.12.0](https://github.com/lumen-oss/lux/compare/v0.11.2...v0.12.0) - 2025-07-31

### Added
- *(cli)* [**breaking**] rename `check` -> `lint` ([#836](https://github.com/lumen-oss/lux/pull/836))

## `lux-cli` - [0.11.2](https://github.com/lumen-oss/lux/compare/v0.11.1...v0.11.2) - 2025-07-30

### Other
- update Cargo.toml dependencies

## `lux-cli` - [0.11.1](https://github.com/lumen-oss/lux/compare/v0.11.0...v0.11.1) - 2025-07-25

### Other
- *(readme)* update feature list

## `lux-cli` - [0.11.0](https://github.com/lumen-oss/lux/compare/v0.10.2...v0.11.0) - 2025-07-23

### Added
- [**breaking**] auto-generate `.luarc.json` ([#910](https://github.com/lumen-oss/lux/pull/910))

### Other
- move shared dependencies to workspace manifest ([#908](https://github.com/lumen-oss/lux/pull/908))

## `lux-cli` - [0.10.2](https://github.com/lumen-oss/lux/compare/v0.10.1...v0.10.2) - 2025-07-23

### Other
- release ([#901](https://github.com/lumen-oss/lux/pull/901))

## `lux-cli` - [0.10.2](https://github.com/lumen-oss/lux/compare/v0.10.1...v0.10.2) - 2025-07-22

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.10.1](https://github.com/lumen-oss/lux/compare/v0.10.0...v0.10.1) - 2025-07-22

### Other
- update flake.lock ([#882](https://github.com/lumen-oss/lux/pull/882))

## `lux-cli` - [0.10.0](https://github.com/lumen-oss/lux/compare/v0.9.1...v0.10.0) - 2025-07-21

### Added
- *(build)* [**breaking**] more output in verbose mode ([#876](https://github.com/lumen-oss/lux/pull/876))

### Fixed
- [**breaking**] support transitive build dependencies ([#883](https://github.com/lumen-oss/lux/pull/883))
- *(cli)* typo in help docs ([#872](https://github.com/lumen-oss/lux/pull/872))

### Other
- *(test-resources)* sample-projects subdirectory

## `lux-cli` - [0.9.1](https://github.com/lumen-oss/lux/compare/v0.9.0...v0.9.1) - 2025-07-15

### Other
- release ([#867](https://github.com/lumen-oss/lux/pull/867))

## `lux-cli` - [0.9.1](https://github.com/lumen-oss/lux/compare/v0.9.0...v0.9.1) - 2025-07-14

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.9.0](https://github.com/lumen-oss/lux/compare/v0.8.3...v0.9.0) - 2025-07-14

### Fixed
- *(build)* [**breaking**] always install and use build dependencies ([#865](https://github.com/lumen-oss/lux/pull/865))
- *(uninstall)* prune dangling dependencies ([#864](https://github.com/lumen-oss/lux/pull/864))
- *(uninstall)* don't uninstall if operation is cancelled
- *(cli)* correct --lua-dir documentation

## `lux-cli` - [0.8.3](https://github.com/lumen-oss/lux/compare/v0.8.2...v0.8.3) - 2025-07-12

### Other
- *(deps)* bump toml from 0.8.22 to 0.9.0 ([#846](https://github.com/lumen-oss/lux/pull/846))

## `lux-cli` - [0.8.2](https://github.com/lumen-oss/lux/compare/v0.8.1...v0.8.2) - 2025-07-08

### Added
- expose shell completions in main binary ([#837](https://github.com/lumen-oss/lux/pull/837))

### Other
- *(cli/completion)* auto-detect shell ([#845](https://github.com/lumen-oss/lux/pull/845))

## `lux-cli` - [0.8.1](https://github.com/lumen-oss/lux/compare/v0.8.0...v0.8.1) - 2025-07-08

### Added
- *(cli)* allow passing path to `fmt` ([#835](https://github.com/lumen-oss/lux/pull/835))

## `lux-cli` - [0.8.0](https://github.com/lumen-oss/lux/compare/v0.7.4...v0.8.0) - 2025-07-07

### Added
- *(cli)* lx shell ([#817](https://github.com/lumen-oss/lux/pull/817))
- add help for `lx lua` flags

### Fixed
- fix!(cli): `lx pack` broken in projects ([#821](https://github.com/lumen-oss/lux/pull/821))

### Other
- [**breaking**] `_prepended` for `PackagePath`
- `lx shell` cleanup
- *(deps)* bump tokio from 1.45.0 to 1.46.0 ([#827](https://github.com/lumen-oss/lux/pull/827))

## `lux-cli` - [0.7.4](https://github.com/lumen-oss/lux/compare/v0.7.3...v0.7.4) - 2025-06-27

### Added
- *(cli)* set `LUA_INIT` for `lx exec`
- feat!(cli): add `--no-loader` flag to repl and run commands

### Fixed
- only run repl initialisation in repl

### Other
- *(deps)* bump lua-src from 547.0.0 to 548.1.1 ([#782](https://github.com/lumen-oss/lux/pull/782))

## `lux-cli` - [0.7.3](https://github.com/lumen-oss/lux/compare/v0.7.2...v0.7.3) - 2025-06-17

### Added
- *(repl)* add project to welcome message

### Fixed
- broken `lx lua --help`

## `lux-cli` - [0.7.2](https://github.com/lumen-oss/lux/compare/v0.7.1...v0.7.2) - 2025-06-16

### Other
- release ([#792](https://github.com/lumen-oss/lux/pull/792))

## `lux-cli` - [0.7.2](https://github.com/lumen-oss/lux/compare/v0.7.1...v0.7.2) - 2025-06-15

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.7.1](https://github.com/lumen-oss/lux/compare/v0.7.0...v0.7.1) - 2025-06-14

### Added
- busted-nlua test backend ([#769](https://github.com/lumen-oss/lux/pull/769))

### Other
- *(licensing)* MIT -> LGPL-3.0+ ([#778](https://github.com/lumen-oss/lux/pull/778))

## `lux-cli` - [0.7.0](https://github.com/lumen-oss/lux/compare/v0.6.0...v0.7.0) - 2025-06-09

### Added
- [**breaking**] `--test` and `--build` flags for `lx lua` ([#774](https://github.com/lumen-oss/lux/pull/774))
- *(cli)* flag to override variables ([#765](https://github.com/lumen-oss/lux/pull/765))

### Fixed
- don't set `LUA_INIT` if lux-lua not present ([#763](https://github.com/lumen-oss/lux/pull/763))

### Other
- *(deps)* bump which from 7.0.3 to 8.0.0 ([#772](https://github.com/lumen-oss/lux/pull/772))
- refactor!(lua-rockspec): split out lua from dependencies ([#730](https://github.com/lumen-oss/lux/pull/730))

## `lux-cli` - [0.6.0](https://github.com/lumen-oss/lux/compare/v0.5.3...v0.6.0) - 2025-06-01

### Added
- feat!(test): full test spec implementation ([#759](https://github.com/lumen-oss/lux/pull/759))
- [**breaking**] lux.toml source templates ([#704](https://github.com/lumen-oss/lux/pull/704))
- add .gitignore to install tree root ([#753](https://github.com/lumen-oss/lux/pull/753))
- keep lux-cli and lux-lua versions in sync ([#751](https://github.com/lumen-oss/lux/pull/751))
- feat!(cli/check): respect ignore files by default ([#749](https://github.com/lumen-oss/lux/pull/749))
- *(cli)* Allow passing args into `lx check` ([#746](https://github.com/lumen-oss/lux/pull/746))

### Fixed
- [**breaking**] more robust lua binary detection ([#757](https://github.com/lumen-oss/lux/pull/757))

## `lux-cli` - [0.5.3](https://github.com/lumen-oss/lux/compare/v0.5.2...v0.5.3) - 2025-05-25

### Fixed
- fix!(build/builtin): use external_dependency info
- properly capture command output

## `lux-cli` - [0.5.2](https://github.com/lumen-oss/lux/compare/v0.5.1...v0.5.2) - 2025-05-21

### Fixed
- unable to parse large luarocks manifest ([#726](https://github.com/lumen-oss/lux/pull/726))

### Other
- *(deps)* upgrade ([#712](https://github.com/lumen-oss/lux/pull/712))

## `lux-cli` - [0.5.1](https://github.com/lumen-oss/lux/compare/v0.5.0...v0.5.1) - 2025-05-16

### Other
- update Cargo.lock dependencies

## `lux-cli` - [0.5.0](https://github.com/lumen-oss/lux/compare/v0.4.5...v0.5.0) - 2025-05-14

### Added
- [**breaking**] separate project from config ([#692](https://github.com/lumen-oss/lux/pull/692))

### Fixed
- [**breaking**] luajit version autodetection + prevent manifest download if lua version not detected ([#702](https://github.com/lumen-oss/lux/pull/702))

### Other
- *(readme)* add packaging status badge ([#698](https://github.com/lumen-oss/lux/pull/698))
- [**breaking**] unify `Install` tree operations

## `lux-cli` - [0.4.5](https://github.com/lumen-oss/lux/compare/v0.4.4...v0.4.5) - 2025-05-13

### Added
- *(cli)* autogenerate a .gitignore file ([#684](https://github.com/lumen-oss/lux/pull/684))

## `lux-cli` - [0.4.1](https://github.com/lumen-oss/lux/compare/v0.4.0...v0.4.1) - 2025-05-11

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.4.0](https://github.com/lumen-oss/lux/compare/v0.3.15...v0.4.0) - 2025-05-10

### Added
- use pkg-config to probe lux-lua
- *(cli)* `lx add` for git dependencies ([#667](https://github.com/lumen-oss/lux/pull/667))

### Other
- [**breaking**] unify `Sync` by making it take in a `Project`

## `lux-cli` - [0.3.15](https://github.com/lumen-oss/lux/compare/v0.3.14...v0.3.15) - 2025-05-09

### Added
- *(cli)* nicer error messages

### Fixed
- *(cli)* rough UX on luajit

### Other
- *(deps)* bump tokio from 1.44.0 to 1.45.0 ([#659](https://github.com/lumen-oss/lux/pull/659))
- add git dependencies to comparison table

## `lux-cli` - [0.3.14](https://github.com/lumen-oss/lux/compare/v0.3.13...v0.3.14) - 2025-05-01

### Added
- git dependencies for local projects ([#644](https://github.com/lumen-oss/lux/pull/644))
- *(lib/install)* support installing from alternate sources ([#624](https://github.com/lumen-oss/lux/pull/624))

### Fixed
- *(build)* dependencies added as install tree entrypoints ([#651](https://github.com/lumen-oss/lux/pull/651))
- *(build)* transitive dependencies added as dependencies of main package

### Other
- refactor!(lux-lib): builder for `PackageInstallSpec` ([#629](https://github.com/lumen-oss/lux/pull/629))

## `lux-cli` - [0.3.13](https://github.com/lumen-oss/lux/compare/v0.3.12...v0.3.13) - 2025-04-29

### Other
- update Cargo.lock dependencies

## `lux-cli` - [0.3.12](https://github.com/lumen-oss/lux/compare/v0.3.11...v0.3.12) - 2025-04-27

### Fixed
- *(cli)* suggest `--no-lock` instead of `--ignore-lockfile`

## `lux-cli` - [0.3.11](https://github.com/lumen-oss/lux/compare/v0.3.10...v0.3.11) - 2025-04-27

### Fixed
- conflicting external dependency spec parse error ([#632](https://github.com/lumen-oss/lux/pull/632))

## `lux-cli` - [0.3.10](https://github.com/lumen-oss/lux/compare/v0.3.9...v0.3.10) - 2025-04-23

### Other
- *(deps)* bump stylua from 2.0.2 to 2.1.0 ([#621](https://github.com/lumen-oss/lux/pull/621))

## `lux-cli` - [0.3.9](https://github.com/lumen-oss/lux/compare/v0.3.8...v0.3.9) - 2025-04-22

### Other
- *(deps)* bump stylua from 2.0.0 to 2.0.2 ([#619](https://github.com/lumen-oss/lux/pull/619))

## `lux-cli` - [0.3.8](https://github.com/lumen-oss/lux/compare/v0.3.7...v0.3.8) - 2025-04-21

### Added
- windows msvc toolchain support ([#501](https://github.com/lumen-oss/lux/pull/501))
- `lx generate-rockspec`

### Fixed
- lockfile entries removed after `lx add` ([#617](https://github.com/lumen-oss/lux/pull/617))

## `lux-cli` - [0.3.7](https://github.com/lumen-oss/lux/compare/v0.3.6...v0.3.7) - 2025-04-16

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.3.6](https://github.com/lumen-oss/lux/compare/v0.3.5...v0.3.6) - 2025-04-14

### Other
- use compilation target to get platform identifier ([#597](https://github.com/lumen-oss/lux/pull/597))

## `lux-cli` - [0.3.5](https://github.com/lumen-oss/lux/compare/v0.3.4...v0.3.5) - 2025-04-14

### Fixed
- *(cli/install-rockspec)* ensure luarocks is installed
- *(build)* wrap binaries ([#583](https://github.com/lumen-oss/lux/pull/583))

## `lux-cli` - [0.3.4](https://github.com/lumen-oss/lux/compare/v0.3.3...v0.3.4) - 2025-04-13

### Other
- updated the following local packages: lux-lib
## `lux-cli` - [0.3.3](https://github.com/lumen-oss/lux/compare/v0.3.2...v0.3.3) - 2025-04-11

### Other
- updated the following local packages: lux-lib
## `lux-cli` - [0.3.2](https://github.com/lumen-oss/lux/compare/v0.3.1...v0.3.2) - 2025-04-10

### Other
- updated the following local packages: lux-lib
## `lux-cli` - [0.3.1](https://github.com/lumen-oss/lux/compare/v0.3.0...v0.3.1) - 2025-04-10

### Other
- update Cargo.lock dependencies
## `lux-cli` - [0.3.0](https://github.com/lumen-oss/lux/compare/v0.2.4...v0.3.0) - 2025-04-08

### Added
- *(debug project)* flag to list included files ([#556](https://github.com/lumen-oss/lux/pull/556))

### Fixed
- [**breaking**] incompatible generated rockspec dependencies

### Other
- make `lx debug`'s description more obvious

## `lux-cli` - [0.2.4](https://github.com/lumen-oss/lux/compare/v0.2.3...v0.2.4) - 2025-04-08

### Fixed
- *(help)* remove [UNIMPLEMENTED] from `lx doc` help

## `lux-cli` - [0.2.3](https://github.com/lumen-oss/lux/compare/v0.2.2...v0.2.3) - 2025-04-07

### Added
- *(build)* flag to build only dependencies

### Fixed
- fix!(sync): lock constraint changes when syncing with project lockfile
- *(build)* project not added to lockfile

## `lux-cli` - [0.2.2](https://github.com/lumen-oss/lux/compare/v0.2.1...v0.2.2) - 2025-04-07

### Other
- updated the following local packages: lux-lib

## `lux-cli` - [0.2.1](https://github.com/lumen-oss/lux/compare/lux-cli-v0.2.0...lux-cli-v0.2.1) - 2025-04-06

### Other
- add `repository` for `lux-cli` so that `cargo binstall` works

## `lux-cli` - [0.2.0](https://github.com/lumen-oss/lux/compare/lux-cli-v0.1.0...lux-cli-v0.2.0) - 2025-04-06

### Added
- implicitly propagate environment variables to subprocesses
- enable vim mode for `lx new` selections
- `lx run` command
- *(`lx new`)* create `src` directory automatically
- *(pin)* operate on lux.toml if in a project ([#486](https://github.com/lumen-oss/lux/pull/486))
- build project on `lx lua` ([#485](https://github.com/lumen-oss/lux/pull/485))
- [**breaking**] allow overriding `etc` tree ([#457](https://github.com/lumen-oss/lux/pull/457))
- feat!(toml): `opt` and `pin` fields ([#456](https://github.com/lumen-oss/lux/pull/456))
- [**breaking**] optional packages ([#453](https://github.com/lumen-oss/lux/pull/453))
- `lux.loader`
- compute hashes for rockspecs dynamically
- *(update)* `--toml` flag to upgrade packages in lux.toml ([#449](https://github.com/lumen-oss/lux/pull/449))
- *(remove)* operate on projects ([#448](https://github.com/lumen-oss/lux/pull/448))
- *(update)* take an optional list of packages ([#446](https://github.com/lumen-oss/lux/pull/446))
- feat!(cli): remove `sync` command
- *(update)* operate on lux.toml and lux.lock if in a project ([#428](https://github.com/lumen-oss/lux/pull/428))

### Fixed
- use compilation target to get platform identifier ([#512](https://github.com/lumen-oss/lux/pull/512))
- `lx run` does not rebuild the project
- *(`lx new`)* don't search parents for existing project ([#493](https://github.com/lumen-oss/lux/pull/493))
- `no such file or directory` when running `lx fmt`
- *(uninstall)* properly handle dependencies

### Other
- turn `run_lua` into an operation
- [**breaking**] rename `lx run` to `lx exec`
- *(deps)* bump octocrab from 0.43.0 to 0.44.0 ([#499](https://github.com/lumen-oss/lux/pull/499))
- *(build)* add case for local project with no source ([#490](https://github.com/lumen-oss/lux/pull/490))
- inconsistent naming in `lx debug project`
- refactor!(toml): extract `LuaDependency` type ([#454](https://github.com/lumen-oss/lux/pull/454))
- prepare flake for new build sequence
- *(deps)* bump tokio from 1.43.0 to 1.44.0 ([#461](https://github.com/lumen-oss/lux/pull/461))
- [**breaking**] introduce `LocalLuaRockspec` and `RemoteLuaRockspec`
- [**breaking**] allow building of local rockspecs
- [**breaking**] break apart `ProjectToml` into `LocalProjectToml` and `RemoteProjectToml`
- [**breaking**] break rockspec apart into `LocalRockspec` and `RemoteRockspec`

