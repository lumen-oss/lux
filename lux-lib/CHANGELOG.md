# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.50.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.49.2...lux-lib-v0.50.0) `lux-lib` - 2026-07-20

### Added
- reduce emojis in progress spinners
- *(tracing)* add trace spans for tokio jobs
- more output on `--verbose`
- instrument build backends + async tasks
- *(tracing)* add fields to spans
- [**breaking**] replace progress bars with `tracing` calls [WIP]

### Fixed
- prevent intermediates compilation from emitting to stderr

### Other
- *(tracing)* make CI happy
- make clippy happy

## [0.49.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.49.1...lux-lib-v0.49.2) `lux-lib` - 2026-07-17

### Dependencies
- *(deps)* bump infer from 0.19.0 to 0.22.0 ([#1696](https://github.com/lumen-oss/lux/pull/1696))

## [0.49.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.49.0...lux-lib-v0.49.1) `lux-lib` - 2026-07-16

### Fixed
- *(logging)* also route MultiProgress output through the sink

## [0.49.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.48.0...lux-lib-v0.49.0) `lux-lib` - 2026-07-16

### Added
- *(error-reporting)* [**breaking**] manifest and project config diagnostics
- *(error-reporting)* [**breaking**] more build module diagnostics

## [0.48.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.47.0...lux-lib-v0.48.0) `lux-lib` - 2026-07-16

### Added
- allow dispatching logging to lua
- *(error-reporting)* [**breaking**] builtin build diagnostics
- *(error-reporting)* [**breaking**] test operations diagnostics
- *(error-reporting)* [**breaking**] lockfile integrity diagnostics

## [0.47.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.46.2...lux-lib-v0.47.0) `lux-lib` - 2026-07-14

### Added
- [**breaking**] improved error messages

## [0.46.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.46.1...lux-lib-v0.46.2) `lux-lib` - 2026-07-13

### Added
- *(build/tree-sitter)* install source queries if none are specified in the rockspec ([#1681](https://github.com/lumen-oss/lux/pull/1681))

## [0.46.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.46.0...lux-lib-v0.46.1) `lux-lib` - 2026-07-12

### Fixed
- *(rockspec)* support per-platform overrides without default ([#1671](https://github.com/lumen-oss/lux/pull/1671))

## [0.46.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.45.1...lux-lib-v0.46.0) `lux-lib` - 2026-07-08

### Added
- *(dist)* single binary project distribution ([#1652](https://github.com/lumen-oss/lux/pull/1652))
- *(rockspec)* [**breaking**] enforce URL in `description.issues_url` ([#1658](https://github.com/lumen-oss/lux/pull/1658))

## [0.45.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.45.0...lux-lib-v0.45.1) `lux-lib` - 2026-07-02

### Added
- *(debug)* add `debug toolchains` subcommand ([#1645](https://github.com/lumen-oss/lux/pull/1645))

## [0.45.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.44.2...lux-lib-v0.45.0) `lux-lib` - 2026-07-02

### Added
- *(upload)* [**breaking**] support two-factor authentication ([#1642](https://github.com/lumen-oss/lux/pull/1642))

## [0.44.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.44.1...lux-lib-v0.44.2) `lux-lib` - 2026-07-02

### Dependencies
- *(deps)* bump nix-nar from 0.4.0 to 0.5.0 ([#1639](https://github.com/lumen-oss/lux/pull/1639))

### Fixed
- *(upload)* generate `-1` specrev, not `-2` if package doesn't exist ([#1644](https://github.com/lumen-oss/lux/pull/1644))

## [0.44.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.44.0...lux-lib-v0.44.1) `lux-lib` - 2026-06-30

### Dependencies
- *(deps)* update ([#1633](https://github.com/lumen-oss/lux/pull/1633))

## [0.44.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.43.3...lux-lib-v0.44.0) `lux-lib` - 2026-06-29

### Added
- *(config)* [**breaking**] add `luarc_file_name` option ([#1626](https://github.com/lumen-oss/lux/pull/1626))

## [0.43.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.43.2...lux-lib-v0.43.3) `lux-lib` - 2026-06-26

### Added
- *(lux-lua)* distribute Lua type definitions ([#1522](https://github.com/lumen-oss/lux/pull/1522))

## [0.43.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.43.1...lux-lib-v0.43.2) `lux-lib` - 2026-06-22

### Added
- *(lua)* build as a DLL on windows ([#1607](https://github.com/lumen-oss/lux/pull/1607))

## [0.43.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.43.0...lux-lib-v0.43.1) `lux-lib` - 2026-06-17

### Other
- update Cargo.toml dependencies

## [0.43.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.42.0...lux-lib-v0.43.0) `lux-lib` - 2026-06-16

### Added
- [**breaking**] remove `NotARemoteGitUrl` error
- *(git)* enable authentication like git CLI ([#1596](https://github.com/lumen-oss/lux/pull/1596))

### Fixed
- *(dependencies)* [**breaking**] improve git remote URL parsing ([#1595](https://github.com/lumen-oss/lux/pull/1595))

## [0.42.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.41.1...lux-lib-v0.42.0) `lux-lib` - 2026-06-15

### Added
- *(dist)* `lx dist flat-archive`
- config option to disable bin script wrapping ([#1586](https://github.com/lumen-oss/lux/pull/1586))
- `InstallProject` operation ([#1585](https://github.com/lumen-oss/lux/pull/1585))

### Fixed
- *(pack)* ensure atomicity of output archive ([#1588](https://github.com/lumen-oss/lux/pull/1588))

## [0.41.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.41.0...lux-lib-v0.41.1) `lux-lib` - 2026-06-05

### Added
- support globs in workplace member paths ([#1580](https://github.com/lumen-oss/lux/pull/1580))

## [0.41.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.40.3...lux-lib-v0.41.0) `lux-lib` - 2026-06-05

### Fixed
- *(fmt)* [**breaking**] allow workspace which resides in a different directory ([#1574](https://github.com/lumen-oss/lux/pull/1574))
- *(build)* [**breaking**] read-only file system error when `src/init.lua` exists ([#1573](https://github.com/lumen-oss/lux/pull/1573))

## [0.40.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.40.2...lux-lib-v0.40.3) `lux-lib` - 2026-06-02

### Dependencies
- *(deps)* bump serial_test from 3.4.0 to 3.5.0 ([#1556](https://github.com/lumen-oss/lux/pull/1556))

## [0.40.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.40.1...lux-lib-v0.40.2) `lux-lib` - 2026-05-31

### Fixed
- *(sync)* don't error on multiple projects with the same dependencies ([#1549](https://github.com/lumen-oss/lux/pull/1549))

## [0.40.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.40.0...lux-lib-v0.40.1) `lux-lib` - 2026-05-28

### Added
- *(generate-lua)* generate long-delimiter multiline printable strings ([#1537](https://github.com/lumen-oss/lux/pull/1537))

## [0.40.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.39.1...lux-lib-v0.40.0) `lux-lib` - 2026-05-27

### Added
- *(gen-rockspec)* [**breaking**] fail if invalid Lua was generated ([#1533](https://github.com/lumen-oss/lux/pull/1533))

### Fixed
- *(generate-lua)* properly escape string contents ([#1534](https://github.com/lumen-oss/lux/pull/1534))

## [0.39.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.39.0...lux-lib-v0.39.1) `lux-lib` - 2026-05-26

### Fixed
- *(test)* only infer dependencies if unspecified ([#1526](https://github.com/lumen-oss/lux/pull/1526))

## [0.39.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.38.3...lux-lib-v0.39.0) `lux-lib` - 2026-05-26

### Added
- support local dependencies ([#1508](https://github.com/lumen-oss/lux/pull/1508))
- support workspaces with multiple projects ([#1503](https://github.com/lumen-oss/lux/pull/1503))

### Other
- update flake.lock ([#1521](https://github.com/lumen-oss/lux/pull/1521))

## [0.38.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.38.2...lux-lib-v0.38.3) `lux-lib` - 2026-05-19

### Added
- *(lua)* restore platform build targets and add freebsd support ([#1514](https://github.com/lumen-oss/lux/pull/1514))

## [0.38.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.38.1...lux-lib-v0.38.2) `lux-lib` - 2026-05-18

### Dependencies
- *(deps)* bump shlex from 1.3.0 to 2.0.1 ([#1512](https://github.com/lumen-oss/lux/pull/1512))

## [0.38.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.38.0...lux-lib-v0.38.1) `lux-lib` - 2026-05-14

### Added
- *(format)* luafmt backend ([#1505](https://github.com/lumen-oss/lux/pull/1505))

## [0.38.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.37.2...lux-lib-v0.38.0) `lux-lib` - 2026-05-09

### Added
- [**breaking**] set user agent header for web requests ([#1481](https://github.com/lumen-oss/lux/pull/1481))

## [0.37.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.37.1...lux-lib-v0.37.2) `lux-lib` - 2026-05-06

### Dependencies
- *(deps)* bump chumsky from 0.12.0 to 0.13.0 ([#1489](https://github.com/lumen-oss/lux/pull/1489))

## [0.37.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.37.0...lux-lib-v0.37.1) `lux-lib` - 2026-05-05

### Added
- *(rockspec)* support non-lua file extensions in `install.lua` ([#1487](https://github.com/lumen-oss/lux/pull/1487))

## [0.37.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.36.4...lux-lib-v0.37.0) `lux-lib` - 2026-05-04

### Dependencies
- *(deps)* bump diffy from 0.4.2 to 0.5.0 ([#1478](https://github.com/lumen-oss/lux/pull/1478))
- *(deps)* bump zip from 8.5.1 to 8.6.0 ([#1476](https://github.com/lumen-oss/lux/pull/1476))

### Fixed
- *(sync)* flipped expected/got in integrity mismatch error

## [0.36.4](https://github.com/lumen-oss/lux/compare/lux-lib-v0.36.3...lux-lib-v0.36.4) `lux-lib` - 2026-04-25

### Other
- update Cargo.toml dependencies

## [0.36.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.36.2...lux-lib-v0.36.3) `lux-lib` - 2026-04-23

### Dependencies
- *(deps)* bump nix-nar from 0.3.1 to 0.4.0 ([#1459](https://github.com/lumen-oss/lux/pull/1459))

## [0.36.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.36.1...lux-lib-v0.36.2) `lux-lib` - 2026-04-15

### Fixed
- *(build/rust-mlua)* drop lib prefix on Windows

## `lux-lib` - [0.36.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.36.0...lux-lib-v0.36.1) - 2026-04-14

### Added
- `no_prompt` option ([#1441](https://github.com/lumen-oss/lux/pull/1441))

## `lux-lib` - [0.36.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.35.0...lux-lib-v0.36.0) - 2026-04-08

### Added
- *(build)* include `.cargo` directory in project files ([#1436](https://github.com/lumen-oss/lux/pull/1436))
- [**breaking**] reduce `ProjectEditError` enum variant size by boxing ([#1435](https://github.com/lumen-oss/lux/pull/1435))
- [**breaking**] remove From<bool> for BuildBehaviour ([#1425](https://github.com/lumen-oss/lux/pull/1425))

### Dependencies
- *(deps)* bump zip from 8.4.0 to 8.5.0 ([#1431](https://github.com/lumen-oss/lux/pull/1431))
- *(deps)* bulk update ([#1420](https://github.com/lumen-oss/lux/pull/1420))

## `lux-lib` - [0.35.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.34.4...lux-lib-v0.35.0) - 2026-03-29

### Dependencies
- *(deps)* bump insta from 1.46.0 to 1.47.0 ([#1417](https://github.com/lumen-oss/lux/pull/1417))
- *(deps)* bump proptest from 1.10.0 to 1.11.0 ([#1412](https://github.com/lumen-oss/lux/pull/1412))

## `lux-lib` - [0.34.4](https://github.com/lumen-oss/lux/compare/lux-lib-v0.34.3...lux-lib-v0.34.4) - 2026-03-24

### Added
- *(sync)* if package files are deleted then redownload the package
- add `lx sync` command

### Dependencies
- *(deps)* bump zip from 8.3.0 to 8.4.0 ([#1405](https://github.com/lumen-oss/lux/pull/1405))
- *(deps)* bump vfs from 0.12.2 to 0.13.0 ([#1401](https://github.com/lumen-oss/lux/pull/1401))
- *(deps)* bump zip from 8.2.0 to 8.3.0 ([#1392](https://github.com/lumen-oss/lux/pull/1392))

## `lux-lib` - [0.34.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.34.2...lux-lib-v0.34.3) - 2026-03-20

### Added
- convert `DisplayAsLuaKV` into a derive macro

## `lux-lib` - [0.34.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.34.1...lux-lib-v0.34.2) - 2026-03-19

### Dependencies
- *(deps)* bump clap from 4.5.60 to 4.6.0 ([#1386](https://github.com/lumen-oss/lux/pull/1386))

### Fixed
- add missing DisplayAsLuaKV implementation for `cargo_extra_args`

## `lux-lib` - [0.34.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.34.0...lux-lib-v0.34.1) - 2026-03-12

### Fixed
- run `harper-cli` on README and fix mistakes

## `lux-lib` - [0.34.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.33.0...lux-lib-v0.34.0) - 2026-03-12

### Added
- port all mlua-specific code to lux-lua
- unify deserialization system to accommodate piccolo
- use `piccolo` for sandboxed evaluation of rockspecs
- hotswap FromLua with Deserialize

### Dependencies
- *(deps)* bump toml and toml_edit ([#1373](https://github.com/lumen-oss/lux/pull/1373))
- *(deps)* bump zip from 8.1.0 to 8.2.0 ([#1365](https://github.com/lumen-oss/lux/pull/1365))

### Other
- move to our fork of piccolo for proper versioning
- update codebase to latest piccolo
- update piccolo to our fork

## `lux-lib` - [0.34.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.33.0...lux-lib-v0.34.0) - 2026-03-12

### Added
- port all mlua-specific code to lux-lua
- unify deserialization system to accommodate piccolo
- use `piccolo` for sandboxed evaluation of rockspecs
- hotswap FromLua with Deserialize

### Dependencies
- *(deps)* bump toml and toml_edit ([#1373](https://github.com/lumen-oss/lux/pull/1373))
- *(deps)* bump zip from 8.1.0 to 8.2.0 ([#1365](https://github.com/lumen-oss/lux/pull/1365))

### Other
- move to our fork of piccolo for proper versioning
- update codebase to latest piccolo
- update piccolo to our fork

## `lux-lib` - [0.33.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.32.2...lux-lib-v0.33.0) - 2026-02-23

### Dependencies
- *(deps)* bump serial_test from 3.3.1 to 3.4.0 ([#1358](https://github.com/lumen-oss/lux/pull/1358))
- *(deps)* bump zip from 8.0.0 to 8.1.0 ([#1351](https://github.com/lumen-oss/lux/pull/1351))
- *(deps)* bump zip from 7.4.0 to 8.0.0 ([#1350](https://github.com/lumen-oss/lux/pull/1350))
- *(deps)* bump bon from 3.8.1 to 3.9.0 ([#1347](https://github.com/lumen-oss/lux/pull/1347))

## `lux-lib` - [0.32.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.32.1...lux-lib-v0.32.2) - 2026-02-09

### Added
- (unofficial) Android compilation target support ([#1338](https://github.com/lumen-oss/lux/pull/1338))

### Other
- *(deps)* bulk update ([#1339](https://github.com/lumen-oss/lux/pull/1339))
- *(readme)* update package badge ([#1336](https://github.com/lumen-oss/lux/pull/1336))
- *(deps)* bump zip from 7.3.0 to 7.4.0 ([#1333](https://github.com/lumen-oss/lux/pull/1333))
- *(deps)* bump proptest from 1.9.0 to 1.10.0 ([#1332](https://github.com/lumen-oss/lux/pull/1332))
- *(deps)* bump zip from 7.2.0 to 7.3.0 ([#1331](https://github.com/lumen-oss/lux/pull/1331))
- *(readme)* add missing Lua 5.5 reference
- *(nix)* don't export `$HOME` ([#1328](https://github.com/lumen-oss/lux/pull/1328))

## `lux-lib` - [0.32.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.32.0...lux-lib-v0.32.1) - 2026-01-30

### Other
- *(readme)* add xtask snippet for Lua 5.5

## `lux-lib` - [0.32.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.31.2...lux-lib-v0.32.0) - 2026-01-29

### Added
- [**breaking**] support lua 5.5 ([#1258](https://github.com/lumen-oss/lux/pull/1258))
- enable luau sandbox mode when loading luarocks manifest
- enable luau sandbox mode when loading remote rockspecs
- enable luau sandbox mode when detecting Lua bin scripts
- enable luau sandbox mode when loading rock manifests
- enable luau sandbox mode when loading `extra.rockspec`
- enable luau sandbox mode when loading rockspecs
- *(lux-cli)* vendor with luau for sandboxing ([#1309](https://github.com/lumen-oss/lux/pull/1309))

### Fixed
- *(build)* substitute variables in rockspec modules ([#1317](https://github.com/lumen-oss/lux/pull/1317))

### Other
- *(readme)* update luacheck url ([#1319](https://github.com/lumen-oss/lux/pull/1319))
- *(build)* simplify Lua script detection ([#1308](https://github.com/lumen-oss/lux/pull/1308))

## `lux-lib` - [0.31.2](https://github.com/lumen-oss/lux/releases/tag/lux-lib-v0.31.2) - 2026-01-21

### Fixed
- *(test/busted-nlua)* unbreak on macOS ([#1304](https://github.com/lumen-oss/lux/pull/1304))

### Other
- release ([#1306](https://github.com/lumen-oss/lux/pull/1306))

## `lux-lib` - [0.31.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.31.1...lux-lib-v0.31.2) - 2026-01-21

### Fixed
- *(test/busted-nlua)* unbreak on macOS ([#1304](https://github.com/lumen-oss/lux/pull/1304))

## `lux-lib` - [0.31.1](https://github.com/lumen-oss/lux/releases/tag/lux-lib-v0.31.1) - 2026-01-21

### Fixed
- *(build/builtin)* pass `LIBFLAG` and `LDFLAGS` to linker ([#1300](https://github.com/lumen-oss/lux/pull/1300))
- *(test/busted-nlua)* ignore lua in `.busted` config file ([#1303](https://github.com/lumen-oss/lux/pull/1303))

### Other
- release ([#1301](https://github.com/lumen-oss/lux/pull/1301))
- *(deps)* bump zip from 7.0.0 to 7.2.0 ([#1299](https://github.com/lumen-oss/lux/pull/1299))

## `lux-lib` - [0.31.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.31.0...lux-lib-v0.31.1) - 2026-01-21

### Fixed
- *(build/builtin)* pass `LIBFLAG` and `LDFLAGS` to linker ([#1300](https://github.com/lumen-oss/lux/pull/1300))
- *(test/busted-nlua)* ignore lua in `.busted` config file ([#1303](https://github.com/lumen-oss/lux/pull/1303))

### Other
- *(deps)* bump zip from 7.0.0 to 7.2.0 ([#1299](https://github.com/lumen-oss/lux/pull/1299))

## `lux-lib` - [0.31.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.30.5...lux-lib-v0.31.0) - 2026-01-17

### Added
- [**breaking**] option to build with vendored directory ([#1283](https://github.com/lumen-oss/lux/pull/1283))
- `vendor` command

## `lux-lib` - [0.30.5](https://github.com/lumen-oss/lux/compare/lux-lib-v0.30.4...lux-lib-v0.30.5) - 2026-01-15

### Other
- split manifest module ([#1287](https://github.com/lumen-oss/lux/pull/1287))

## `lux-lib` - [0.30.4](https://github.com/lumen-oss/lux/compare/lux-lib-v0.30.3...lux-lib-v0.30.4) - 2026-01-12

### Fixed
- *(build)* better inferring of `source.dir` ([#1279](https://github.com/lumen-oss/lux/pull/1279))

### Other
- *(deps)* bump insta from 1.45.0 to 1.46.0 ([#1276](https://github.com/lumen-oss/lux/pull/1276))
- *(deps)* bump serde-enum-str from 0.4.0 to 0.5.0 ([#1274](https://github.com/lumen-oss/lux/pull/1274))
- *(deps)* bump serial_test from 3.2.0 to 3.3.1 ([#1272](https://github.com/lumen-oss/lux/pull/1272))
- enable install_binary_rock test only on x86_64-linux ([#1181](https://github.com/lumen-oss/lux/pull/1181))

## `lux-lib` - [0.30.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.30.2...lux-lib-v0.30.3) - 2025-12-24

### Added
- improved logic for `source.dir` auto-detection ([#1262](https://github.com/lumen-oss/lux/pull/1262))
- better error message on copy lua module failure ([#1261](https://github.com/lumen-oss/lux/pull/1261))

### Other
- *(deps)* bump zip from 6.0.0 to 7.0.0 ([#1255](https://github.com/lumen-oss/lux/pull/1255))
- *(deps)* bump insta from 1.44.3 to 1.45.0 ([#1254](https://github.com/lumen-oss/lux/pull/1254))
- *(deps)* bump chumsky from 0.11.2 to 0.12.0 ([#1251](https://github.com/lumen-oss/lux/pull/1251))

## `lux-lib` - [0.30.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.30.1...lux-lib-v0.30.2) - 2025-12-12

### Fixed
- compilation error due to unused import

### Other
- *(deps)* update tree-sitter

## `lux-lib` - [0.30.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.30.0...lux-lib-v0.30.1) - 2025-12-12

### Other
- *(deps)* bump tree-sitter-config from 0.25.10 to 0.26.2 ([#1242](https://github.com/lumen-oss/lux/pull/1242))

## `lux-lib` - [0.30.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.29.0...lux-lib-v0.30.0) - 2025-12-06

### Fixed
- *(exec)* [**breaking**] always build project first ([#1236](https://github.com/lumen-oss/lux/pull/1236))

## `lux-lib` - [0.29.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.28.5...lux-lib-v0.29.0) - 2025-12-03

### Added
- [**breaking**] various error type and message improvements ([#1229](https://github.com/lumen-oss/lux/pull/1229))
- better error message on project lua version mismatch ([#1228](https://github.com/lumen-oss/lux/pull/1228))
- dependencies update + better error messages when failing to parse rockspec ([#1202](https://github.com/lumen-oss/lux/pull/1202))

### Other
- [**breaking**] disallow panic, expect and unwrap ([#1223](https://github.com/lumen-oss/lux/pull/1223))

## `lux-lib` - [0.28.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.28.2...lux-lib-v0.28.3) - 2025-11-30

### Fixed
- *(upload)* broken detection of existing package version
- *(upload)* fail if luarocks.org retruns non-OK status on check
- improve version detection when generating rockspec

### Other
- *(upload)* make `rock_exists` private
- fix clippy warnings
- *(deps)* bump insta from 1.43.2 to 1.44.0 ([#1203](https://github.com/lumen-oss/lux/pull/1203))
- *(deps)* bump bytes from 1.10.1 to 1.11.0 ([#1199](https://github.com/lumen-oss/lux/pull/1199))

## `lux-lib` - [0.28.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.28.1...lux-lib-v0.28.2) - 2025-11-10

### Fixed
- *(install)* quote Lua executable path in bin wrappers ([#1194](https://github.com/lumen-oss/lux/pull/1194))

## `lux-lib` - [0.28.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.28.0...lux-lib-v0.28.1) - 2025-11-05

### Added
- *(git)* add ssh auth callback ([#1152](https://github.com/lumen-oss/lux/pull/1152))

### Fixed
- *(install-lua)* confusing error message when dependency is missing ([#1187](https://github.com/lumen-oss/lux/pull/1187))

## `lux-lib` - [0.28.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.27.2...lux-lib-v0.28.0) - 2025-11-04

### Fixed
- don't generate test table in rockspec ([#1179](https://github.com/lumen-oss/lux/pull/1179))
- *(sync)* prevent trying to uninstall from wrong tree

### Other
- *(uninstall)* [**breaking**] use `bon::Builder`

## `lux-lib` - [0.27.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.27.1...lux-lib-v0.27.2) - 2025-11-03

### Fixed
- *(test)* correct test executable in error message

## `lux-lib` - [0.27.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.26.5...lux-lib-v0.27.0) - 2025-11-02

### Added
- [**breaking**] use `org.lumenlabs.lux` as identifier ([#1163](https://github.com/lumen-oss/lux/pull/1163))

### Fixed
- *(cli/shell)* install Lua version if missing
- *(cli/lua)* install Lua version if missing

## `lux-lib` - [0.26.4](https://github.com/lumen-oss/lux/compare/lux-lib-v0.26.3...lux-lib-v0.26.4) - 2025-10-29

### Other
- *(deps)* bump proptest from 1.8.0 to 1.9.0 ([#1146](https://github.com/lumen-oss/lux/pull/1146))

## `lux-lib` - [0.26.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.26.2...lux-lib-v0.26.3) - 2025-10-21

### Added
- *(run_lua)* detect if lux-lua is available in a lua wrapper ([#1141](https://github.com/lumen-oss/lux/pull/1141))

## `lux-lib` - [0.26.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.26.0...lux-lib-v0.26.1) - 2025-10-15

### Fixed
- *(windows)* unset readonly attribute before cleaning .git directory ([#1124](https://github.com/lumen-oss/lux/pull/1124))

## `lux-lib` - [0.26.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.25.2...lux-lib-v0.26.0) - 2025-10-15

### Added
- [**breaking**] more detailed error messages when failing to fetch sources ([#1121](https://github.com/lumen-oss/lux/pull/1121))

## `lux-lib` - [0.25.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.25.1...lux-lib-v0.25.2) - 2025-10-12

### Fixed
- 'not a tag' panic when generating rockspec ([#1116](https://github.com/lumen-oss/lux/pull/1116))

### Other
- *(deps)* bump bon to 3.8.0 ([#1104](https://github.com/lumen-oss/lux/pull/1104))
- *(deps)* bump zip from 5.1.1 to 6.0.0 ([#1111](https://github.com/lumen-oss/lux/pull/1111))
- update README to reflect .luarc generation

## `lux-lib` - [0.25.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.25.0...lux-lib-v0.25.1) - 2025-09-24

### Fixed
- don't download remote manifest if is there is nothing to install ([#1091](https://github.com/lumen-oss/lux/pull/1091))
- *(lua)* failure to install when path contains spaces ([#1085](https://github.com/lumen-oss/lux/pull/1085))

### Other
- update homepage and PKGBUILD conflicts
- *(deps)* bump proptest from 1.7.0 to 1.8.0 ([#1084](https://github.com/lumen-oss/lux/pull/1084))
- *(readme)* add note about statically linking gpgme

## `lux-lib` - [0.25.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.24.1...lux-lib-v0.25.0) - 2025-09-17

### Added
- [**breaking**] don't expose `git-url-parse` types ([#1073](https://github.com/lumen-oss/lux/pull/1073))

### Other
- *(deps)* replace unmaintained `tempdir` with `tempfile` ([#1074](https://github.com/lumen-oss/lux/pull/1074))

## `lux-lib` - [0.24.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.24.0...lux-lib-v0.24.1) - 2025-09-15

### Other
- *(deps)* bulk update ([#1066](https://github.com/lumen-oss/lux/pull/1066))
- *(docs)* rename nvim-neorocks -> lumen-oss ([#1057](https://github.com/lumen-oss/lux/pull/1057))
- *(deps)* bump zip from 5.0.0 to 5.1.0 ([#1056](https://github.com/lumen-oss/lux/pull/1056))
- *(deps)* bump zip from 4.6.0 to 5.0.0 ([#1053](https://github.com/lumen-oss/lux/pull/1053))
- *(deps)* bump zip from 4.5.0 to 4.6.0 ([#1048](https://github.com/lumen-oss/lux/pull/1048))
- *(readme)* exclude unsupported repos in packaging status badge

## `lux-lib` - [0.21.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.20.0...lux-lib-v0.21.0) - 2025-08-14

### Other
- *(operations)* [**breaking**] rename `Remove` to `Uninstall` for consistent terminology ([#795](https://github.com/lumen-oss/lux/pull/795))

## `lux-lib` - [0.20.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.19.3...lux-lib-v0.20.0) - 2025-08-14

### Other
- [**breaking**] binary package distributions ([#877](https://github.com/lumen-oss/lux/pull/877))

## `lux-lib` - [0.19.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.19.2...lux-lib-v0.19.3) - 2025-08-13

### Fixed
- *(lua)* improvements to detecting/installing ([#976](https://github.com/lumen-oss/lux/pull/976))

### Other
- *(deps)* bump bon from 3.6.3 to 3.7.0 ([#971](https://github.com/lumen-oss/lux/pull/971))

## `lux-lib` - [0.19.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.19.0...lux-lib-v0.19.1) - 2025-08-05

### Added
- `lx check` command for luaCATS typechecks ([#849](https://github.com/lumen-oss/lux/pull/849))

## `lux-lib` - [0.19.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.18.1...lux-lib-v0.19.0) - 2025-08-03

### Added
- *(lockfile)* diff-friendly entrypoints ([#948](https://github.com/lumen-oss/lux/pull/948))
- *(build)* [**breaking**] pass in `LuaInstallation`
- [**breaking**] manage lua installations internally

### Other
- *(build)* [**breaking**] tidy up builder

## `lux-lib` - [0.18.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.17.0...lux-lib-v0.18.0) - 2025-07-31

### Added
- *(cli)* [**breaking**] rename `check` -> `lint` ([#836](https://github.com/lumen-oss/lux/pull/836))

## `lux-lib` - [0.17.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.16.1...lux-lib-v0.17.0) - 2025-07-30

### Fixed
- *(rockspec)* [**breaking**] support list elements in install spec ([#939](https://github.com/lumen-oss/lux/pull/939))

## `lux-lib` - [0.16.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.16.0...lux-lib-v0.16.1) - 2025-07-25

### Added
- expand variables in `source` `.dir`, `.file` and `.tag` ([#915](https://github.com/lumen-oss/lux/pull/915))

### Fixed
- *(build)* support multiline scripts in `command` backend ([#918](https://github.com/lumen-oss/lux/pull/918))
- search upwards when substituting `$(REF)` ([#917](https://github.com/lumen-oss/lux/pull/917))

### Other
- *(readme)* update feature list

## `lux-lib` - [0.16.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.15.1...lux-lib-v0.16.0) - 2025-07-23

### Added
- [**breaking**] auto-generate `.luarc.json` ([#910](https://github.com/lumen-oss/lux/pull/910))

### Other
- move shared dependencies to workspace manifest ([#908](https://github.com/lumen-oss/lux/pull/908))

## `lux-lib` - [0.15.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.15.0...lux-lib-v0.15.1) - 2025-07-23

### Added
- pretty-print generated lua code ([#907](https://github.com/lumen-oss/lux/pull/907))
- more detailed error message when variable substitution fails ([#905](https://github.com/lumen-oss/lux/pull/905))

## `lux-lib` - [0.14.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.14.0...lux-lib-v0.14.1) - 2025-07-22

### Fixed
- incorrect install path when installing packed rock ([#896](https://github.com/lumen-oss/lux/pull/896))
- fall back to unzipped manifest on HEAD request ([#895](https://github.com/lumen-oss/lux/pull/895))

### Other
- *(deps)* bump nonempty from 0.11.0 to 0.12.0 ([#894](https://github.com/lumen-oss/lux/pull/894))
- update flake.lock ([#882](https://github.com/lumen-oss/lux/pull/882))
- clarify cross-compilation comment

## `lux-lib` - [0.14.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.13.2...lux-lib-v0.14.0) - 2025-07-21

### Added
- fall back to unzipped manifest ([#890](https://github.com/lumen-oss/lux/pull/890))
- *(build)* [**breaking**] more output in verbose mode ([#876](https://github.com/lumen-oss/lux/pull/876))

### Fixed
- *(pack)* write `rock_manifest` using luarocks structure ([#887](https://github.com/lumen-oss/lux/pull/887))
- *(install)* don't install build dependencies of binary rocks ([#888](https://github.com/lumen-oss/lux/pull/888))
- [**breaking**] support transitive build dependencies ([#883](https://github.com/lumen-oss/lux/pull/883))

### Other
- *(test-resources)* sample-projects subdirectory
- *(deps)* bump mlua from 0.10.3 to 0.10.5 ([#875](https://github.com/lumen-oss/lux/pull/875))

## `lux-lib` - [0.13.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.13.1...lux-lib-v0.13.2) - 2025-07-15

### Fixed
- *(lux.toml)* bad conversion of deploy spec to lua ([#871](https://github.com/lumen-oss/lux/pull/871))

## `lux-lib` - [0.13.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.12.1...lux-lib-v0.13.0) - 2025-07-14

### Fixed
- *(build)* [**breaking**] always install and use build dependencies ([#865](https://github.com/lumen-oss/lux/pull/865))

## `lux-lib` - [0.12.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.12.0...lux-lib-v0.12.1) - 2025-07-12

### Fixed
- *(build)* relax `source.dir` inferring logic ([#859](https://github.com/lumen-oss/lux/pull/859))
- *(config)* TOML configs overridden by defaults ([#858](https://github.com/lumen-oss/lux/pull/858))

### Other
- *(deps)* bump zip from 4.2.0 to 4.3.0 ([#850](https://github.com/lumen-oss/lux/pull/850))
- *(deps)* bump toml_edit from 0.22.26 to 0.23.0 ([#847](https://github.com/lumen-oss/lux/pull/847))
- *(deps)* bump toml from 0.8.22 to 0.9.0 ([#846](https://github.com/lumen-oss/lux/pull/846))

## `lux-lib` - [0.12.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.11.0...lux-lib-v0.12.0) - 2025-07-08

### Fixed
- *(build)* [**breaking**] `copy_directorys` drops subdirectories ([#842](https://github.com/lumen-oss/lux/pull/842))
- *(build)* install conf files to etc/conf ([#841](https://github.com/lumen-oss/lux/pull/841))

## `lux-lib` - [0.11.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.10.1...lux-lib-v0.11.0) - 2025-07-07

### Added
- *(lux-lua)* state functions and search functionality ([#781](https://github.com/lumen-oss/lux/pull/781))
- use `--verbose` flag to enable compiler warnings ([#833](https://github.com/lumen-oss/lux/pull/833))
- *(install)* support rocks with only .src.rock sources ([#823](https://github.com/lumen-oss/lux/pull/823))

### Fixed
- fix!(cli): `lx pack` broken in projects ([#821](https://github.com/lumen-oss/lux/pull/821))
- *(build/command)* make `_command` fields optional ([#832](https://github.com/lumen-oss/lux/pull/832))

### Other
- *(build)* [**breaking**] don't expose `BuildBackend` trait ([#826](https://github.com/lumen-oss/lux/pull/826))
- *(build)* [**breaking**] use Builder pattern for `BuildBackend` trait ([#825](https://github.com/lumen-oss/lux/pull/825))
- [**breaking**] `_prepended` for `PackagePath`
- *(build)* [**breaking**] `lua_rockspec::Build` -> `build::backend::BuildBackend` ([#824](https://github.com/lumen-oss/lux/pull/824))
- *(deps)* bump tokio from 1.45.0 to 1.46.0 ([#827](https://github.com/lumen-oss/lux/pull/827))

## `lux-lib` - [0.10.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.10.0...lux-lib-v0.10.1) - 2025-06-27

### Added
- *(cli)* set `LUA_INIT` for `lx exec`
- feat!(cli): add `--no-loader` flag to repl and run commands

### Fixed
- only run repl initialisation in repl

### Other
- *(deps)* bump md5 from 0.7.0 to 0.8.0 ([#816](https://github.com/lumen-oss/lux/pull/816))
- *(deps)* bump zip from 4.1.0 to 4.2.0 ([#814](https://github.com/lumen-oss/lux/pull/814))
- *(deps)* bump lua-src from 547.0.0 to 548.1.1 ([#782](https://github.com/lumen-oss/lux/pull/782))

## `lux-lib` - [0.10.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.9.2...lux-lib-v0.10.0) - 2025-06-17

### Added
- *(repl)* add project to welcome message

### Fixed
- [**breaking**] only alias `exit` to `os.exit()` in repl

### Other
- *(deps)* bump zip from 4.0.0 to 4.1.0 ([#800](https://github.com/lumen-oss/lux/pull/800))

## `lux-lib` - [0.9.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.9.0...lux-lib-v0.9.1) - 2025-06-14

### Added
- busted-nlua test backend ([#769](https://github.com/lumen-oss/lux/pull/769))
- *(rockspec)* support `gitrec+` prefixes ([#786](https://github.com/lumen-oss/lux/pull/786))

### Other
- *(licensing)* MIT -> LGPL-3.0+ ([#778](https://github.com/lumen-oss/lux/pull/778))
- *(cargo.toml)* use repository instead of homepage ([#779](https://github.com/lumen-oss/lux/pull/779))

## `lux-lib` - [0.9.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.8.0...lux-lib-v0.9.0) - 2025-06-09

### Added
- [**breaking**] `--test` and `--build` flags for `lx lua` ([#774](https://github.com/lumen-oss/lux/pull/774))
- *(cli)* flag to override variables ([#765](https://github.com/lumen-oss/lux/pull/765))

### Fixed
- fix!(install): properly link transitive dependencies ([#771](https://github.com/lumen-oss/lux/pull/771))
- properly quote complex keys when generating rockspec
- don't set `LUA_INIT` if lux-lua not present ([#763](https://github.com/lumen-oss/lux/pull/763))
- *(build)* lua binaries not wrapped properly ([#766](https://github.com/lumen-oss/lux/pull/766))

### Other
- *(deps)* bump proptest from 1.6.0 to 1.7.0 ([#776](https://github.com/lumen-oss/lux/pull/776))
- *(deps)* bump which from 7.0.3 to 8.0.0 ([#772](https://github.com/lumen-oss/lux/pull/772))
- refactor!(lua-rockspec): split out lua from dependencies ([#730](https://github.com/lumen-oss/lux/pull/730))

## `lux-lib` - [0.8.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.7.0...lux-lib-v0.8.0) - 2025-06-01

### Added
- feat!(test): full test spec implementation ([#759](https://github.com/lumen-oss/lux/pull/759))
- [**breaking**] lux.toml source templates ([#704](https://github.com/lumen-oss/lux/pull/704))
- substitute variables from environment
- [**breaking**] make `HasVariables` trait `pub(crate)`
- add .gitignore to install tree root ([#753](https://github.com/lumen-oss/lux/pull/753))
- feat!(cli/check): respect ignore files by default ([#749](https://github.com/lumen-oss/lux/pull/749))

### Fixed
- [**breaking**] more robust lua binary detection ([#757](https://github.com/lumen-oss/lux/pull/757))
- [**breaking**] always wrap lua bin scripts ([#756](https://github.com/lumen-oss/lux/pull/756))

### Other
- follow-up fix for source url templates

## `lux-lib` - [0.7.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.6.2...lux-lib-v0.7.0) - 2025-05-25

### Fixed
- fix!(build/builtin): use external_dependency info
- properly capture command output
- variable substitution for `LUA_BINDIR`
- external_dependencies variable substitutions
- rock_manifest parsing error
- external dependencies not finding libraries via pkg-config
- fall back to `all.rock` when downloading packed rocks
- add checks to prevent trying to unpack HTML response ([#735](https://github.com/lumen-oss/lux/pull/735))
- [**breaking**] bin scripts installed into tree's root ([#724](https://github.com/lumen-oss/lux/pull/724))

### Other
- add gnum4 to devShell
- *(deps)* bump zip from 3.0.0 to 4.0.0 ([#728](https://github.com/lumen-oss/lux/pull/728))

## `lux-lib` - [0.6.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.6.1...lux-lib-v0.6.2) - 2025-05-21

### Fixed
- unable to parse large luarocks manifest ([#726](https://github.com/lumen-oss/lux/pull/726))

### Other
- *(deps)* upgrade ([#712](https://github.com/lumen-oss/lux/pull/712))

## `lux-lib` - [0.6.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.6.0...lux-lib-v0.6.1) - 2025-05-16

### Fixed
- error when luajit is not aliased to lua ([#707](https://github.com/lumen-oss/lux/pull/707))

### Other
- *(deps)* bump zip from 2.6.0 to 3.0.0 ([#705](https://github.com/lumen-oss/lux/pull/705))

## `lux-lib` - [0.6.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.5.0...lux-lib-v0.6.0) - 2025-05-14

### Added
- more detailed zip error messages ([#701](https://github.com/lumen-oss/lux/pull/701))
- [**breaking**] separate project from config ([#692](https://github.com/lumen-oss/lux/pull/692))

### Fixed
- [**breaking**] luajit version autodetection + prevent manifest download if lua version not detected ([#702](https://github.com/lumen-oss/lux/pull/702))
- remove hash field from `RockSourceInternal` ([#697](https://github.com/lumen-oss/lux/pull/697))

### Other
- *(readme)* add packaging status badge ([#698](https://github.com/lumen-oss/lux/pull/698))
- [**breaking**] unify `Install` tree operations

## `lux-lib` - [0.5.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.4.1...lux-lib-v0.5.0) - 2025-05-13

### Fixed
- [**breaking**] treat unknown string versions as `< SemVer` ([#689](https://github.com/lumen-oss/lux/pull/689))
# Changelog

All notable changes to this project will be documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## `lux-lib` - [0.4.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.12...lux-lib-v0.4.0) - 2025-05-10

### Added
- *(cli)* `lx update` for git dependencies ([#671](https://github.com/lumen-oss/lux/pull/671))
- use pkg-config to probe lux-lua
- *(cli)* `lx add` for git dependencies ([#667](https://github.com/lumen-oss/lux/pull/667))

### Fixed
- *(project)* use project tree instead of tree provided in configuration
- *(cli)* fields removed on update

### Other
- [**breaking**] unify `Sync` by making it take in a `Project`

## `lux-lib` - [0.3.12](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.11...lux-lib-v0.3.12) - 2025-05-09

### Added
- more Lua coverage + Lua tests

### Fixed
- *(cli)* rough UX on luajit

### Other
- *(deps)* bump tokio from 1.44.0 to 1.45.0 ([#659](https://github.com/lumen-oss/lux/pull/659))
- add git dependencies to comparison table
- *(deps)* bump luajit-src from 210.5.11+97813fb to 210.5.12+a4f56a4 ([#656](https://github.com/lumen-oss/lux/pull/656))

## `lux-lib` - [0.3.11](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.10...lux-lib-v0.3.11) - 2025-05-01

### Added
- git dependencies for local projects ([#644](https://github.com/lumen-oss/lux/pull/644))
- *(lib/install)* support installing from alternate sources ([#624](https://github.com/lumen-oss/lux/pull/624))

### Fixed
- *(build)* dependencies added as install tree entrypoints ([#651](https://github.com/lumen-oss/lux/pull/651))
- *(build)* unpacking tar archive can panic ([#649](https://github.com/lumen-oss/lux/pull/649))

### Other
- refactor!(lux-lib): builder for `PackageInstallSpec` ([#629](https://github.com/lumen-oss/lux/pull/629))

## `lux-lib` - [0.3.10](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.9...lux-lib-v0.3.10) - 2025-04-29

### Added
- *(lux-lib)* more lenient dev version parsing ([#623](https://github.com/lumen-oss/lux/pull/623))

### Fixed
- parse versions without a contraint prefix as == ([#640](https://github.com/lumen-oss/lux/pull/640))

### Other
- *(deps)* bump insta from 1.42.0 to 1.43.0 ([#642](https://github.com/lumen-oss/lux/pull/642))

## `lux-lib` - [0.3.9](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.8...lux-lib-v0.3.9) - 2025-04-27

### Fixed
- conflicting external dependency spec parse error ([#632](https://github.com/lumen-oss/lux/pull/632))

## `lux-lib` - [0.3.8](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.7...lux-lib-v0.3.8) - 2025-04-21

### Added
- windows msvc toolchain support ([#501](https://github.com/lumen-oss/lux/pull/501))

### Fixed
- *(manifest)* re-download if corrupted

### Other
- update flake.lock ([#615](https://github.com/lumen-oss/lux/pull/615))

## `lux-lib` - [0.3.6](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.5...lux-lib-v0.3.6) - 2025-04-14

### Fixed
- *(pack)* regression in manifest creation ([#599](https://github.com/lumen-oss/lux/pull/599))

### Other
- use compilation target to get platform identifier ([#597](https://github.com/lumen-oss/lux/pull/597))

## `lux-lib` - [0.3.5](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.4...lux-lib-v0.3.5) - 2025-04-14

### Added
- better dev version parsing
- better variable expansion + error on missing variables

### Fixed
- install pre-packaged luarocks on windows ([#584](https://github.com/lumen-oss/lux/pull/584))
- *(build)* wrap binaries ([#583](https://github.com/lumen-oss/lux/pull/583))

### Other
- *(deps)* bump bon from 3.5.0 to 3.6.0 ([#586](https://github.com/lumen-oss/lux/pull/586))

## `lux-lib` - [0.3.1](https://github.com/lumen-oss/lux/compare/lux-lib-v0.3.0...lux-lib-v0.3.1) - 2025-04-10

### Fixed
- `[run]` field overwritten by `extra.rockspec` ([#566](https://github.com/lumen-oss/lux/pull/566))
- unsupported off-spec `install.bin` array field

## `lux-lib` - [0.3.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.2.3...lux-lib-v0.3.0) - 2025-04-08

### Added
- *(debug project)* flag to list included files ([#556](https://github.com/lumen-oss/lux/pull/556))

### Fixed
- *(build)* properly handle legacy rockspecs ([#557](https://github.com/lumen-oss/lux/pull/557))
- [**breaking**] incompatible generated rockspec dependencies

## `lux-lib` - [0.2.3](https://github.com/lumen-oss/lux/compare/lux-lib-v0.2.2...lux-lib-v0.2.3) - 2025-04-08

### Fixed
- *(rockspec)* support undocumented string/array duality

## `lux-lib` - [0.2.2](https://github.com/lumen-oss/lux/compare/lux-lib-v0.2.1...lux-lib-v0.2.2) - 2025-04-07

### Fixed
- fix!(sync): lock constraint changes when syncing with project lockfile

## `lux-lib` - [0.2.0](https://github.com/lumen-oss/lux/compare/lux-lib-v0.1.0...lux-lib-v0.2.0) - 2025-04-06

### Added
- implicitly propagate environment variables to subprocesses
- `lx run` command
- add `operations::run`
- *(`lux.toml`)* add `[run]` support
- *(pin)* operate on lux.toml if in a project ([#486](https://github.com/lumen-oss/lux/pull/486))
- *(build)* respect ignore files when copying source ([#495](https://github.com/lumen-oss/lux/pull/495))
- [**breaking**] allow overriding `etc` tree ([#457](https://github.com/lumen-oss/lux/pull/457))
- feat!(toml): `opt` and `pin` fields ([#456](https://github.com/lumen-oss/lux/pull/456))
- [**breaking**] optional packages ([#453](https://github.com/lumen-oss/lux/pull/453))
- `lux.loader`
- Lua API
- *(build)* treesitter-parser build backend ([#452](https://github.com/lumen-oss/lux/pull/452))
- compute hashes for rockspecs dynamically
- *(update)* `--toml` flag to upgrade packages in lux.toml ([#449](https://github.com/lumen-oss/lux/pull/449))
- *(remove)* operate on projects ([#448](https://github.com/lumen-oss/lux/pull/448))
- *(update)* take an optional list of packages ([#446](https://github.com/lumen-oss/lux/pull/446))
- allow `--tree` to override project tree ([#432](https://github.com/lumen-oss/lux/pull/432))
- *(update)* operate on lux.toml and lux.lock if in a project ([#428](https://github.com/lumen-oss/lux/pull/428))

### Fixed
- use compilation target to get platform identifier ([#512](https://github.com/lumen-oss/lux/pull/512))
- do not include `lua` as part of dependencies in TOML rockspecs
- map between luarocks and semver versions ([#483](https://github.com/lumen-oss/lux/pull/483))
- *(build)* don't fall back to `.src.rock` for local sources ([#494](https://github.com/lumen-oss/lux/pull/494))
- *(`lx new`)* don't search parents for existing project ([#493](https://github.com/lumen-oss/lux/pull/493))
- disallow `lua` in `dependencies` field
- *(uninstall)* properly handle dependencies
- *(build)* copy_directories into etc subdirectories ([#462](https://github.com/lumen-oss/lux/pull/462))
- minimize extraneous compiler output


### Other
- *(deps)* bump zip from 2.5.0 to 2.6.0 ([#514](https://github.com/lumen-oss/lux/pull/514))
- turn `run_lua` into an operation
- [**breaking**] rename `lx run` to `lx exec`
- *(deps)* bump zip from 2.4.1 to 2.5.0 ([#492](https://github.com/lumen-oss/lux/pull/492))
- *(deps)* bump zip from 2.3.0 to 2.4.1
- refactor!(toml): extract `LuaDependency` type ([#454](https://github.com/lumen-oss/lux/pull/454))
- *(lockfile)* hide unnecessarily public structs/methods
- [**breaking**] remove `lua` cargo feature
- *(nix)* fix `nix flake check`
- prepare flake for new build sequence
- [**breaking**] name all lockfiles `lux.lock`
- *(deps)* bump zip from 2.2.0 to 2.3.0 ([#470](https://github.com/lumen-oss/lux/pull/470))
- *(deps)* bump bon from 3.4.0 to 3.5.0 ([#469](https://github.com/lumen-oss/lux/pull/469))
- *(deps)* bump tokio from 1.43.0 to 1.44.0 ([#461](https://github.com/lumen-oss/lux/pull/461))
- *(deps)* bump bon from 3.3.2 to 3.4.0 ([#455](https://github.com/lumen-oss/lux/pull/455))
- [**breaking**] introduce `LocalLuaRockspec` and `RemoteLuaRockspec`
- *(`rocks pack`)* [**breaking**] disallow paths to `lux.toml` files
- [**breaking**] allow building of local rockspecs
- [**breaking**] break apart `ProjectToml` into `LocalProjectToml` and `RemoteProjectToml`
- [**breaking**] break rockspec apart into `LocalRockspec` and `RemoteRockspec`
- use crane for clippy and rustfmt checks ([#450](https://github.com/lumen-oss/lux/pull/450))
- *(deps)* bump flate2 from 1.0.35 to 1.1.0 ([#438](https://github.com/lumen-oss/lux/pull/438))

