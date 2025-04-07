<h1 align="center">
  <a href="https://nvim-neorocks.github.io/tutorial/getting-started"><img src="./lux-logo.svg" alt="Lux" width="200"></a>
  <br>
  <h1 align="center">Lux</h1>
</h1>

<h4 align="center">A luxurious package manager for <a href="https://www.lua.org/" target="_blank">Lua</a>.</h4>

<p align="center">
  <img alt="GitHub Actions Workflow Status" src="https://img.shields.io/github/actions/workflow/status/nvim-neorocks/lux/integration-tests.yml?style=for-the-badge">
  <img alt="GitHub top language" src="https://img.shields.io/github/languages/top/nvim-neorocks/lux?style=for-the-badge">
  <img alt="GitHub License" src="https://img.shields.io/github/license/nvim-neorocks/lux?style=for-the-badge">
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
  - Powered by [stylua](https://github.com/JohnnyMorganz/StyLua).
* Easily specify compatible Lua versions
  - Lux will take care of Lua header installation automatically
  - Forget about users complaining they have the wrong Lua headers installed on their system
* Automatic code linting via `lx check`
  - Powered by `luacheck`.
* Powerful lockfile support
  - Makes for fully reproducible developer environments.
  - Makes Lux easy to integrate with Nix!
* Fully compatible
  - Works with existing luarocks packages.
  - Have a complex rockspec that you don't want to rewrite to TOML? No problem!
    Lux allows the creation of an `extra.rockspec` file, everything just works.
  - Have a very complex build script? Lux can shell out to `luarocks` if it knows it has
    to preserve maximum compatibility.

> [!WARNING]
>
> **Lux, while generally functional, is a work in progress
> and does not have a `1.0` release yet.**

## :gear: How To Use

Feel free to consult the [documentation](https://nvim-neorocks.github.io/tutorial/getting-started) on how to get started with Lux!

It features a tutorial and several guides to make you good at managing Lua projects.

## :bar_chart: Comparison with `luarocks`

As this project is still a work in progress, some luarocks features
have not been (fully) implemented yet.
On the other hand, lux has some features that are not present in luarocks.

The following table provides a brief comparison:

|                                                                       | lux                          | luarocks v3.11.1   |
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
| static type checking                                                  | :x: (planned)                | :x:                |

[^1]: Supported via a compatibility layer that uses luarocks as a backend.
[^2]: [You Aren't Gonna Need It.](https://martinfowler.com/bliki/Yagni.html)
[^3]: Mostly compatible with the luarocks version parser,
      which allows an arbitrary number of version components.
      To comply with SemVer, we treat anything after the third version component
      (except for the specrev) as a prerelease/build version.

## :bulb: You may also like...

- [luarocks](https://github.com/luarocks/luarocks) - The original Lua package manager
- [rocks.nvim](https://github.com/nvim-neorocks/rocks.nvim) - A Neovim plugin manager that uses `luarocks` under the hood, and will soon be undergoing a rewrite to use Lux instead.

## :purple_heart: Credits

Credits go to the Luarocks team for maintaining [luarocks](https://github.com/luarocks/luarocks) and [luarocks.org](https://luarocks.org) for as long as they have.
Without their prior work Lux would not be possible.

## :green_heart: Contributing

Contributions are more than welcome!
See [CONTRIBUTING.md](./CONTRIBUTING.md) for a guide.

## :book: License

- Lux is licensed under [MIT](./LICENSE).
- The Lux logo © 2025 by Kai Jakobi is licensed under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).
