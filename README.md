<h1 align="center">
  <br>
  <a href="https://lumen-oss.github.io/tutorial/getting-started"><img src="./lux-logo.svg" alt="Lux" width="200"></a>
  <br>
  <br>
  <span>Lux</span>
</h1>

<h4 align="center">A luxurious package manager for <a href="https://www.lua.org/" target="_blank">Lua</a>.</h4>

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

* Create and manage Lua projects
  - Easily manage dependencies, build steps and more through the `lux.toml` file.
* Parallel builds and installs :rocket:
* Add/remove dependencies with simple CLI commands
* Automatic generation of rockspecs
  - Say goodbye to managing 10 different rockspec files in your source code :tada:
* Integrated code formatting via `lx fmt`
  - Powered by [`stylua`](https://github.com/JohnnyMorganz/StyLua).
* Easily specify compatible Lua versions
  - Lux will take care of Lua header installation automatically
  - Forget about users complaining they have the wrong Lua headers installed on their system
* Automatic EmmyLua/LuaCATS based type checking via `lx check`
  - Powered by [`emmylua-analyzer-rust`](https://github.com/EmmyLuaLs/emmylua-analyzer-rust)
* Automatic code linting via `lx lint`
  - Powered by [`luacheck`](https://github.com/mpeterv/luacheck)
* Powerful lockfile support
  - Makes for fully reproducible developer environments.
  - Makes Lux easy to integrate with Nix!
* Fully compatible
  - Works with existing luarocks packages.
  - Have a complex rockspec that you don't want to rewrite to TOML? No problem!
    Lux allows the creation of an `extra.rockspec` file, everything just works.
  - Have a very complex build script? Lux can shell out to `luarocks` if it knows it has
    to preserve maximum compatibility.
* Automatically adds project dependencies to a [`.luarc.json`](https://luals.github.io/wiki/configuration/#luarcjson-file)
  file so they can be picked up by [`lua-language-server`](https://luals.github.io/).

> [!WARNING]
>
> **Lux, while generally functional, is a work in progress
> and does not have a `1.0` release yet.**

## :gear: How To Use

Feel free to consult the [documentation](https://lumen-oss.github.io/tutorial/getting-started) on how to get started with Lux!

It features a tutorial and several guides to make you good at managing Lua projects.

## :bar_chart: Comparison with `luarocks`

As this project is still a work in progress, some luarocks features
have not been (fully) implemented yet.
On the other hand, lux has some features that are not present in luarocks.

The following table provides a brief comparison:

|                                                                       | lux                          | luarocks v3.12.2   |
| ---                                                                   | ---                          | ---                |
| project format                                                        | TOML / Lua                   | Lua                |
| add/remove dependencies                                               | :white_check_mark:           | :x:                |
| parallel builds/installs                                              | :white_check_mark:           | :x:                |
| proper lockfile support with integrity checks                         | :white_check_mark:           | :x: (basic, dependency versions only) |
| run tests with busted                                                 | :white_check_mark:           | :white_check_mark: |
| linting with luacheck                                                 | :white_check_mark:           | :x:                |
| code formatting with stylua                                           | :white_check_mark:           | :x:                |
| automatic lua detection/installation                                  | :white_check_mark:           | :x:                |
| default build specs                                                   | :white_check_mark:           | :white_check_mark: |
| custom build backends                                                 | :white_check_mark:[^1]       | :white_check_mark: |
| `rust-mlua` build spec                                                | :white_check_mark: (builtin) | :white_check_mark: (external build backend) |
| `treesitter-parser` build spec                                        | :white_check_mark: (builtin) | :white_check_mark: (external build backend) |
| install pre-built binary rocks                                        | :white_check_mark:           | :white_check_mark: |
| install multiple packages with a single command                       | :white_check_mark:           | :x:                |
| install packages using version constraints                            | :white_check_mark:           | :x:                |
| auto-detect external dependencies and Lua headers with `pkg-config`   | :white_check_mark:           | :x:                |
| resolve multiple versions of the same dependency at runtime           | :white_check_mark:           | :white_check_mark: |
| pack and upload pre-built binary rocks                                | :white_check_mark:           | :white_check_mark: |
| luarocks.org manifest namespaces                                      | :white_check_mark:           | :white_check_mark: |
| luarocks.org dev packages                                             | :white_check_mark:           | :white_check_mark: |
| versioning                                                            | SemVer[^3]                   | arbitrary          |
| rockspecs with CVS/Mercurial/SVN/SSCM sources                         | :x: (YAGNI[^2])              | :white_check_mark: |
| static type checking                                                  | :white_check_mark:           | :x:                |
| generate a `.luarc` file with dependencies                            | :x: (planned)                | :x:                |
| git dependencies in local projects                                    | :white_check_mark:           | :x:                |

[^1]: Supported via a compatibility layer that uses luarocks as a backend.
[^2]: [You Aren't Gonna Need It.](https://martinfowler.com/bliki/Yagni.html)
[^3]: Mostly compatible with the luarocks version parser,
      which allows an arbitrary number of version components.
      To comply with SemVer, we treat anything after the third version component
      (except for the specrev) as a prerelease/build version.

## :package: Packages

<a href="https://repology.org/project/lux-cli/versions">
    <img src="https://repology.org/badge/vertical-allrepos/lux-cli.svg?header=lux-cli&exclude_unsupported=1" alt="lux-cli packaging status" align="right">
</a>

<a href="https://repology.org/project/lux-lua-unclassified/versions">
    <img src="https://repology.org/badge/vertical-allrepos/lux-lua-unclassified.svg?header=lux-lua&exclude_unsupported=1" alt="lux-lua packaging status" align="right">
</a>

Lux includes the following packages and libraries:

- `lux-cli`: The main CLI for interacting with projects and installing Lua packages
  from the command line.

- `lux-lua`: The Lux Lua API, which provides:
  - `lux.loader` for resolving dependencies on `require` at runtime.
  - A work-in-progress API for embedding Lux into Lua applications.
  We provide builds of `lux-lua` for Lua 5.1, 5.2, 5.3, 5.4 and Luajit.
  `lux-cli` uses `lux-lua` for commands like `lx lua`, `lx run` and `lx path`.

- `lux-lib`: The Lux library for Rust. A dependency of `lux-cli` and `lux-lua`.

> [!NOTE]
>
> We do not yet provide a way to install `lux-lua` as a Lua library using Lux.
> See [#663](https://github.com/lumen-oss/lux/issues/663).
> Lux can detect a lux-lua installation using pkg-config
> or via the `LUX_LIB_DIR` environment variable.
>
> Our [pre-built binary release artifacts](https://github.com/lumen-oss/lux/releases/latest)
> are bundled with `lux-lua`.

## :wrench: Building from source

Dependencies:

- `gnupg`, `libgpg-error` and `gpgme` (*nix only)

If building without the `vendored` feature:

- `lua`
- `libgit2`
- `openssl`

If building with the `vendored` feature:

- `perl` and `perl-core`
- `make`

We usually recommend building with the `vendored` feature enabled,
to statically link `lua`, `libgit2` and `openssl`:

```bash
cargo build --locked --profile release --features vendored
```

Or, to build with dynamically linked libraries:

```bash
cargo build --locked --profile release
```

On Windows/MSVC, you must disable the `gpgme` feature:

```bash
cargo build --locked --profile release --no-default-features --features lua54,vendored
```

You can build `lux-lua` for a given Lua version with:

```bash
cargo xtask51 dist-lua # lux-lua for Lua 5.1
cargo xtask52 dist-lua # for Lua 5.2
cargo xtask53 dist-lua # ...
cargo xtask54 dist-lua
cargo xtaskjit dist-lua
```

This will install `lux-lua` to `target/dist/share/lux-lua/<lua>/lux.so`
and a pkg-config `.pc` file to `target/dist/lib/lux-lua*.pc`.

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

If you would like to use the latest version of lux with Nix,
you can import our flake.
It provides an overlay and packages for:

- `lux-cli`: The Lux CLI package.
- `lux-lua51` The Lux Lua API for Lua 5.1
- `lux-lua52` The Lux Lua API for Lua 5.2
- `lux-lua53` The Lux Lua API for Lua 5.3
- `lux-lua54` The Lux Lua API for Lua 5.4
- `lux-luajit` The Lux Lua API for Luajit

If you have a `lux-lua` build and `pkg-config` in a Nix devShell,
Lux will auto-detect `lux-lua`.

## :bulb: You may also like...

- [luarocks](https://github.com/luarocks/luarocks) - The original Lua package manager
- [rocks.nvim](https://github.com/lumen-oss/rocks.nvim) - A Neovim plugin manager that uses `luarocks` under the hood, and will soon be undergoing a rewrite to use Lux instead.

## :purple_heart: Credits

Credits go to the Luarocks team for maintaining [luarocks](https://github.com/luarocks/luarocks) and [luarocks.org](https://luarocks.org) for as long as they have.
Without their prior work Lux would not be possible.

## :green_heart: Contributing

Contributions are more than welcome!
See [CONTRIBUTING.md](./CONTRIBUTING.md) for a guide.

## :book: License

- Lux is licensed under [LGPL-3.0+](./LICENSE).
- The Lux logo © 2025 by Kai Jakobi is licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).
