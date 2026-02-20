# Contributing guide

Contributions are more than welcome!

We label issues that we think should be easy for first-time contributors
with [`good-first-issue`](https://github.com/lumen-oss/lux/issues?q=is%3Aissue%20state%3Aopen%20label%3A%22good%20first%20issue%22).

This document assumes that you already know how to use GitHub and Git.
If that's not the case, we recommend learning about it first [here](https://docs.github.com/en/get-started/quickstart/hello-world).

## Strict No LLM / No AI Policy

No LLMs for issues.

No LLMs for pull requests.

No LLMs for comments on issues/PRs, including translation. English is encouraged, but not required. You are welcome to post in your native language and rely on others to have their own translation tools of choice to interpret your words.

## Creating pull requests

Please ensure your pull request title conforms to [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/),
as they may end up in our [generated changelog](./CHANGELOG.md).

> [!TIP]
>
> Look at our commit history for some examples for conventional commit
> prefixes and scopes we've used.

## CI

Our CI checks are run using [`nix`](https://nixos.org/download.html#download-nix).

## Development

You don't have to use `nix`.
But we recommend it, because it is easier to reproduce CI build/test failures
and our [nix dev shell](#nix-dev-shell) provides everything you need
to build Lux out of the box.

### Dev environment

See [README.md/Building from source](./README.md#wrench-building-from-source)
for build dependencies.
We use the following tools:

#### Formatting

- [`rustfmt`](https://github.com/rust-lang/rustfmt) [Rust]
- [`alejandra`](https://github.com/kamadorueda/alejandra) [Nix]

#### Linting

- [`cargo check`](https://doc.rust-lang.org/cargo/commands/cargo-check.html)
- [`clippy`](https://doc.rust-lang.org/clippy/)

### Nix dev shell

- Requires [flakes](https://nixos.wiki/wiki/Flakes) to be enabled.

For Nix users, we provide a `devShell` that can bootstrap
everything needed to build, format and lint Lux.

To enter a development shell:

```console
nix develop
```

To apply formatting and run linters, run

```console
nix fmt
```

Optionally, you can use [`direnv`](https://direnv.net/) to auto-load
the development shell. Just run `direnv allow`.

### Testing

We value proof of functionality in the form of automated tests.
For that, we rely on

- Unit/property tests, in a `tests` submodule of the respective modules to be tested.
- Integration tests: Modules in a `tests` directory.

We recommend [test driven development](https://martinfowler.com/bliki/TestDrivenDevelopment.html),
but that is up to you.

> [!NOTE]
>
> Impure unit tests that require a network connection cannot be built with Nix.
> So we try to avoid them if possible (this is not the case for integration tests,
> which we don't run with Nix).
> If a unit test absolutely needs a network connection, please make sure to skip
> it if the `LUX_SKIP_IMPURE_TESTS` environment variable is set to 1.

#### Running tests without nix

For reproducibility, we only run tests that can be sandboxed with `nix`,
skipping integration tests and impure tests that need a network connection.

Running `cargo test` locally will run all tests, including integration tests.

Or, if you are using [cargo-nextest](https://nexte.st/), we provide an alias:

```bash
cargo tt
```

#### Running tests and checks with Nix

If you just want to run all checks that are available, run:

```console
nix flake check -Lv
```

To run individual checks, using Nix:

```console
nix build .#checks.<your-system>.<check> -Lv
```

For example:

```console
nix build .#checks.x86_64-linux.tests -Lv
```

For formatting and linting:

```console
nix build .#checks.<your-system>.git-hooks-check -Lv
```

#### Testing Lux manually

For convenience, we provide a `cargo lx` alias,
which will build Lux in debug mode and invoke its CLI with any arguments
you pass to it.

Example:

```conseole
cargo lx --help
```

> [!NOTE]
>
> If your contribution was tested manually, we will likely ask you to write some tests :)
