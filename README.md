<h1 align="center">
  <br>
  <a href="https://lux.lumen-labs.org/tutorial/getting-started"><img src="./lux-logo.svg" alt="Lux" width="200"></a>
  <br>
  <br>
  <span>Lux</span>
</h1>

<h4 align="center">A luxurious package manager for <a href="https://www.lua.org/" target="_blank">Lua</a> and <a href="https://luau.org" target="_blank">Luau</a>.</h4>

<p align="center">
  <img alt="GitHub Actions Workflow Status" src="https://img.shields.io/github/actions/workflow/status/lumen-oss/lux/integration-tests.yml?style=for-the-badge">
  <img alt="GitHub Top Language" src="https://img.shields.io/github/languages/top/lumen-oss/lux?style=for-the-badge">
  <img alt="GitHub License" src="https://img.shields.io/github/license/lumen-oss/lux?style=for-the-badge">
</p>

<p align="center">
  <a href="#star2-key-features">Key Features</a> •
  <a href="#gear-how-to-use">How To Use</a> •
  <a href="#bar_chart-comparison-with-luarocks">Comparison with Luarocks</a> •
  <a href="#bulb-you-may-also-like">Related Projects</a> •
  <a href="#green_heart-contributing">Contributing</a>
</p>

## :star2: Key Features

Lux is a next-generation package manager with ease of use and security in mind. It removes all the troubles
of building, managing and deploying Lua packages and brings joy back into the Lua development cycle.

#### :package: Create and Manage Lua Projects

- Easily manage your entire project in a single, unified `lux.toml` file.
- Supports everything a modern ecosystem needs - build scripts, dependencies, testing and more.
- Full support for the `rockspec` file format.

#### :gear: Modern, Comfortable Tooling

- Integrated code formatting, type checking and powerful code linting.
- Good DX - automatically adds project dependencies to [`.luarc.json`](https://luals.github.io/wiki/configuration/#luarcjson-file) for rich LSP diagnostics.
- Distribute projects (including dependencies) as install tree archives
  or as static binaries that **just work** on target machines.

#### :tada: Automatic Rockspec Generation

- Lux handles versioning and rockspec generation for you.
- Say goodbye to managing 10 different rockspec files in your source code!

#### :jigsaw: Lua Versioning Done Easy

- Lux takes care of installing the right versions of Lua headers automatically.
- Forget about users complaining they have the wrong Lua installed on their system.

#### :lock: Advanced Lockfile Support

- Fully reproducible developer environments.
- Makes Lux easy to integrate with Nix.

#### :globe_with_meridians: Fully Ecosystem Compatible

- Works with existing Luarocks packages.
- Have a complex rockspec that you don't want to rewrite to TOML?
  Lux allows the creation of an `extra.rockspec` file for just that purpose.
- Have a very complex build script? Lux can utilise `luarocks` as a library if it detects it has
  to preserve maximum compatibility.

## :gear: How To Use

Feel free to consult the [documentation](https://lux.lumen-labs.org/tutorial/getting-started) on how to get started with Lux!

It features a tutorial and several guides to make you good at managing Lua projects.

> [!NOTE]
>
> **Lux, while fully functional, is a work in progress
> and does not have a `1.0` release yet.**

## :bar_chart: Comparison with Luarocks

As this project is still a work in progress, some Luarocks features
have not been fully implemented yet.
On the other hand, Lux has some features that are not present in Luarocks.

The following table provides a brief comparison:

|                                                                                         | Lux                          | Luarocks v3.13.0   |
| ---                                                                                     | ---                          | ---                |
| Project format                                                                          | TOML / Lua                   | Lua                |
| Add/remove dependencies                                                                 | :white_check_mark:           | :x:                |
| Parallel builds/installs                                                                | :white_check_mark:           | :x:                |
| Proper lockfile support with integrity checks                                           | :white_check_mark:           | :x: (basic, dependency versions only) |
| Run tests with busted                                                                   | :white_check_mark:           | :white_check_mark: |
| Linting with luacheck                                                                   | :white_check_mark:           | :x:                |
| Code formatting with stylua, luafmt, or emmylua_codestyle                               | :white_check_mark:           | :x:                |
| Automatic lua detection/installation                                                    | :white_check_mark:           | :x:                |
| Default build specs                                                                     | :white_check_mark:           | :white_check_mark: |
| Custom build backends                                                                   | :white_check_mark:[^1]       | :white_check_mark: |
| `rust-mlua` build spec                                                                  | :white_check_mark: (builtin) | :white_check_mark: (external build backend) |
| `treesitter-parser` build spec                                                          | :white_check_mark: (builtin) | :white_check_mark: (external build backend) |
| Install prebuilt binary rocks                                                           | :white_check_mark:           | :white_check_mark: |
| Install multiple packages with a single command                                         | :white_check_mark:           | :x:                |
| Install packages using version constraints                                              | :white_check_mark:           | :x:                |
| Auto-detect external dependencies and Lua headers with `pkg-config`                     | :white_check_mark:           | :x:                |
| Resolve multiple versions of the same dependency at runtime                             | :white_check_mark:           | :white_check_mark: |
| Pack and upload prebuilt binary rocks                                                   | :white_check_mark:           | :white_check_mark: |
| Luarocks.org manifest namespaces                                                        | :white_check_mark:           | :white_check_mark: |
| Luarocks.org dev packages                                                               | :white_check_mark:           | :white_check_mark: |
| Versioning                                                                              | SemVer[^2]                   | arbitrary          |
| Rockspecs with CVS/Mercurial/SVN/SSCM sources                                           | :x: (YAGNI[^3])              | :white_check_mark: |
| Load installed packages from multiple install trees                                     | :x: ([#1493](https://github.com/lumen-oss/lux/issues/1493)) | :white_check_mark: |
| Static type checking                                                                    | :white_check_mark:           | :x:                |
| Automatic Lua detection                                                                 | :white_check_mark:           | :x:                |
| Luau support                                                                            | :white_check_mark:           | :x:                |
| Generate a `.luarc` file with dependencies                                              | :white_check_mark:           | :x:                |
| Git dependencies in local projects                                                      | :white_check_mark:           | :x:                |
| Local dependencies in local projects                                                    | :white_check_mark:           | :x:                |
| Multiple projects in a single workspace                                                 | :white_check_mark:           | :x:                |
| Vendor sources for offline use                                                          | :white_check_mark:           | :x:                |
| Distribute install tree archives                                                        | :white_check_mark:           | :x:                |
| Distribute static binaries                                                              | :white_check_mark:           | :x:                |
| Load RockSpecs and Luarocks manifests with [full sandboxing](https://github.com/kyren/piccolo) | :white_check_mark:    | :x:                |
| Sandbox build execution                                                                 | :white_check_mark: [^4]      | :x:                |

[^1]: Supported via a compatibility layer that uses Luarocks as a backend.
[^2]: Aims to be compatible with the Luarocks version parser,
      which allows an arbitrary number of version components.
      We treat anything after the third version component
      (except for the specrev) as a prerelease/build version,
      but allow comparing versions with/without prerelease components.
[^3]: [You Aren't Gonna Need It.](https://martinfowler.com/bliki/Yagni.html)
[^4]: Opt-in via a `build.runner` config.

## :package: Packages

<a href="https://repology.org/project/lux-lua-package-manager/versions">
    <img src="https://repology.org/badge/vertical-allrepos/lux-lua-package-manager.svg?header=lux&exclude_unsupported=1" alt="lux packaging status" align="right">
</a>

Lux includes the following packages and libraries:

- `lux-cli`: The main CLI for interacting with projects and installing Lua packages
  from the command line.

- `lux-lua`: The Lux Lua API, which provides:
  - `lux.loader` for resolving dependencies on `require` at runtime.
  - A work-in-progress API for embedding Lux into Lua applications.
  We provide builds of `lux-lua` for Lua 5.1, 5.2, 5.3, 5.4, 5.5 and LuaJIT.
  `lux-cli` uses `lux-lua` for commands like `lx lua`, `lx run` and `lx path`.

- `lux-lib`: The Lux library for Rust. A dependency of `lux-cli` and `lux-lua`.

> [!NOTE]
>
> Lux can detect a lux-lua installation using [pkg-config](https://www.freedesktop.org/wiki/Software/pkg-config/)
> or [pkgconf](http://pkgconf.org/).
>
> Our [prebuilt binary release artifacts](https://github.com/lumen-oss/lux/releases/latest)
> are bundled with `lux-lua`.

## :wrench: Building from source

Dependencies:

- `gnupg`, `libgpg-error` and `gpgme` (*nix only)

If building without the `vendored` feature:

- `libgit2`
- `openssl`

If building with the `vendored` feature:

- `perl` and `perl-core`
- `make`

To link `gpgme` statically on Linux and macOS, set the environment variable
`SYSTEM_DEPS_LINK=static`.

On macOS, Homebrew's `libgpg-error` is built with `gettext`, which is not part of
the system libraries. The `cargo xtask build` and `cargo xtask dist-package`
tasks automatically install a static, NLS-disabled build of `libgpg-error` when
`SYSTEM_DEPS_LINK=static` is set. To build it manually:

```bash
cargo xtask build-static-libgpg-error
PKG_CONFIG_PATH="$PWD/target/static-libgpg-error/prefix/lib/pkgconfig" \
  SYSTEM_DEPS_LINK="static" cargo build --locked --profile release --features vendored
```

We usually recommend building with the `vendored` feature enabled,
to statically link `libgit2` and `openssl`:

```bash
SYSTEM_DEPS_LINK="static" cargo build --locked --profile release --features vendored
```

Or, to build with dynamically linked libraries:

```bash
cargo build --locked --profile release
```

On Windows/MSVC, you must disable the `gpgme` feature:

```bash
cargo build --locked --profile release --no-default-features --features vendored
```

You can build `lux-lua` for a given Lua version with:

```bash
cargo xtask-lua51 dist # lux-lua for Lua 5.1
cargo xtask-lua52 dist # for Lua 5.2
cargo xtask-lua53 dist # ...
cargo xtask-lua54 dist
cargo xtask-lua55 dist
cargo xtask-luajit dist
```

This will install `lux-lua` to `target/dist/share/lux-lua/<lua>/lux.so`
and a pkg-config `.pc` file to `target/dist/lib/lux-lua*.pc`.

By default, the `$prefix` in the generated `.pc` file is `${{pcfiledir}}/../../share/lux-lua/`.
You can override it by setting `LUX_LIB_DIR`, for example:

```bash
LUX_LIB_DIR="/usr/lib/lua/5.1" cargo xtask-lua51 dist
```

To build completions:

```bash
cargo xtask dist-completions
```

To build man pages:

```bash
cargo xtask dist-man
```

To build the binary distributions for your platform,
bundled with completions, man pages and `lux-lua`:

```bash
cargo xtask dist-package
```

## :snowflake: Nix flake

If you would like to use the latest version of Lux with Nix,
you can import our flake.
It provides an overlay and packages for:

- `lux-cli`: The Lux CLI package.
- `lux-lua51` The Lux Lua API for Lua 5.1
- `lux-lua52` The Lux Lua API for Lua 5.2
- `lux-lua53` The Lux Lua API for Lua 5.3
- `lux-lua54` The Lux Lua API for Lua 5.4
- `lux-lua55` The Lux Lua API for Lua 5.5
- `lux-luajit` The Lux Lua API for LuaJIT

If you have a `lux-lua` build and `pkg-config` in a Nix devShell,
Lux will auto-detect `lux-lua`.

## :bulb: You may also like...

- [Luarocks](https://github.com/luarocks/luarocks) - The original Lua package manager
- [rocks.nvim](https://github.com/lumen-oss/rocks.nvim) - A Neovim plugin manager that uses `luarocks` under the hood, and will soon be undergoing a rewrite to use Lux instead.

## :purple_heart: Credits

Credits go to the Luarocks team for maintaining [Luarocks](https://github.com/luarocks/luarocks) and [luarocks.org](https://luarocks.org) for as long as they have.
Without their prior work Lux would not be possible.

## :green_heart: Contributing

Contributions are more than welcome!
See [CONTRIBUTING.md](./CONTRIBUTING.md) for a guide.

## :book: License

- Lux is licensed under [LGPL-3.0+](./LICENSE).
- The Lux logo © 2025 by Kai Jakobi is licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).
- The nix helper functions in the `nix/` are licensed under the
  MIT No Attribution License (MIT-0).
