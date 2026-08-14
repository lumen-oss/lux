# Lux {#lux}

> [!WARNING]
>
> The helper functions described in this readme are unstable.
> They are a playground intended for experimentation and possible upstreaming
> to nixpkgs, and may change significantly.

The helper functions in this section build Lux packages with Nix.

## Using Lux packages {#using-lux-packages}

### Building a Lux package {#building-a-lux-package}

`buildLuxPackage` builds a project that has a `lux.toml` manifest:

```nix
{
  buildLuxPackage,
  fetchFromGitHub,
  lib,
  lua5_1,
  lux-lua51,
}:

buildLuxPackage {
  lua = lua5_1;
  lux-lua = lux-lua51;
} {
  pname = "fallo";
  version = "0-unstable-2026-03-25";

  src = fetchFromGitHub {
    owner = "NTBBloodbath";
    repo = "fallo";
    rev = "c2efe9ad31c97265afb696b62e90312a6b944dc2";
    hash = "sha256-imyKEDvbn9EWJ8Cm54v5sUKZV2fZuBTT3du+1ipKgW4=";
  };

  luxHash = "sha256-DpkDMK6AV7Iy6YFbKbx+EEkDeGaAFbszNkLPhdVr+qY=";

  meta = {
    description = "Modern and ergonomic error handling for Lua";
    homepage = "https://github.com/NTBBloodbath/fallo";
    license = lib.licenses.lgpl21Plus;
  };
}
```

`buildLuxPackage` takes the Lua interpreter and the `lux-lua` library as its first argument and the package attributes as its second argument.
It installs the package and its dependencies into a Lux tree at `$out/share/lux`.

### Building a Lux application {#building-a-lux-application}

`buildLuxApplication` wraps `buildLuxPackage` and produces an executable.
It reads the `[run]` table from `lux.toml` to determine the interpreter and arguments:

```nix
buildLuxApplication {
  lua = lua5_1;
  lux-lua = lux-lua51;
} {
  pname = "fallo";
  version = "0-unstable-2026-03-25";

  src = fetchFromGitHub {
    owner = "NTBBloodbath";
    repo = "fallo";
    rev = "c2efe9ad31c97265afb696b62e90312a6b944dc2";
    hash = "sha256-imyKEDvbn9EWJ8Cm54v5sUKZV2fZuBTT3du+1ipKgW4=";
  };

  luxHash = "sha256-DpkDMK6AV7Iy6YFbKbx+EEkDeGaAFbszNkLPhdVr+qY=";
}
```

Override the run arguments with `runArgs` and the interpreter with `runCommand`.

### Building a package from a RockSpec {#building-a-package-from-a-rockspec}

`buildLuxRockspec` builds a package from a RockSpec instead of a `lux.toml`.
Declare the package's Lua dependencies with `propagatedBuildInputs`:

```nix
buildLuxRockspec {
  lua = lua5_1;
  lux-lua = lux-lua51;
} {
  pname = "lua-cjson";
  version = "2.1.0.10-1";

  src = fetchFromGitHub {
    owner = "openresty";
    repo = "lua-cjson";
    tag = "2.1.0.10";
    hash = "sha256-/SeQro0FaJn91bAGjsVIin+mJF89VUm/G0KyJkV9Qps=";
  };

  knownRockspec = fetchurl {
    url = "mirror://luarocks/lua-cjson-2.1.0.10-1.rockspec";
    hash = "sha256-r+WGLILja827kVC0giYvUXOmvQeQhRb9bJN0cXA+Vxc=";
  };
}
```

If the source ships with the RockSpec, omit `knownRockspec` and the derivation looks for `<pname>-<version>.rockspec` in `src`.

### Building a Neovim plugin {#building-a-neovim-plugin}

Build the package with `buildLuxPackage`, then convert it with `toLuxNeovimPlugin`.
The plugin's runtime files are exposed at the plugin root and its Lua modules are resolved by the `lux.loader`:

```nix
{
  buildLuxPackage,
  fetchFromGitHub,
  lib,
  luajit,
  lux-luajit,
  toLuxNeovimPlugin,
}:

let
  plugin = buildLuxPackage {
    lua = luajit;
    lux-lua = lux-luajit;
  } {
    pname = "my-plugin.nvim";
    version = "1.0.0";

    src = fetchFromGitHub {
      owner = "example";
      repo = "my-plugin.nvim";
      rev = "v1.0.0";
      hash = lib.fakeHash;
    };

    luxHash = lib.fakeHash;
  };
in
toLuxNeovimPlugin plugin
```

Add the result to Neovim with `neovim.override { plugins = [ ... ]; }` or `vimPlugins`.
The `neovim` and `lua.withPackages` overrides from the Lux overlay activate lux-lua's `lux.loader`.

## Vendoring dependencies {#vendoring-dependencies}

Lux builds without network access, so `buildLuxPackage` needs the package's dependencies at build time.
Pass exactly one of the following:

- `luxHash`: a hash for the fixed-output derivation that vendors the dependencies.
- `luxDeps`: a pre-fetched vendor directory, for example from `importLuxLock`.
- `luxLock`: a path to a `lux.lock` file, imported with `importLuxLock`.
- `luxVendorDir`: a directory with vendored sources.

`luxHash` vendors the dependencies from luarocks.org with `fetchLuxDeps`.
Set it to `lib.fakeHash`, build, and copy the correct hash from the error message.

`importLuxLock` fetches the dependencies listed in a `lux.lock` file without a hash:

```nix
{
  buildLuxPackage,
  fetchFromGitHub,
  importLuxLock,
  lib,
  lua5_1,
  lux-lua51,
}:

buildLuxPackage {
  lua = lua5_1;
  lux-lua = lux-lua51;
} {
  pname = "fallo";
  version = "0-unstable-2026-03-25";

  src = fetchFromGitHub {
    owner = "NTBBloodbath";
    repo = "fallo";
    rev = "c2efe9ad31c97265afb696b62e90312a6b944dc2";
    hash = lib.fakeHash;
  };

  luxDeps = importLuxLock {
    lockFile = ./lux.lock;
  };
}
```

Use `importLuxLock` when the project commits its `lux.lock`.
It fetches each dependency as a separate fixed-output derivation and does not require updating a hash after changing the lock file.

## Lux Reference {#lux-reference}

### `buildLuxPackage` function {#buildluxpackage-function}

`buildLuxPackage` accepts the following arguments:

- `pname`, `version`, `src`: the package name, version, and source.
- `luxHash`, `luxDeps`, `luxLock`, `luxVendorDir`: one of the dependency sources described in [Vendoring dependencies](#vendoring-dependencies).
- `luxRoot`: a subdirectory of `src` that contains `lux.toml`.
- `buildAndTestSubdir`: a subdirectory of `src` in which to build and run tests.
- `nvim`: build the package with the Neovim layout. Equivalent to `lx --nvim`.

It also accepts the standard `stdenv.mkDerivation` attributes, such as `nativeBuildInputs`, `propagatedBuildInputs`, and `doCheck`.

### `buildLuxRockspec` function {#buildluxrockspec-function}

`buildLuxRockspec` accepts the following arguments:

- `pname`, `version`, `src`: the package name, version, and source.
- `knownRockspec`: a RockSpec file to use instead of `<pname>-<version>.rockspec` from `src`.
- `rockspecVersion`: the version used in the RockSpec filename, defaults to `version`.
- `propagatedBuildInputs`: the package's Lua dependencies, marked with `toLuaModule`.
- `nvim`: build the package with a Neovim plugin layout.

### `toLuxNeovimPlugin` function {#toluxneovimplugin-function}

`toLuxNeovimPlugin` converts a package built with `buildLuxPackage` or `buildLuxRockspec` into a Neovim plugin.
It overrides the package to use the Neovim plugin layout, exposes the runtime files at the plugin root, and sets `passthru.vimPlugin = true`.
