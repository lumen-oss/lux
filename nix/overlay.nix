{
  self,
  crane,
}: final: prev: let
  lib = final.lib;
  craneLib = crane.mkLib prev;

  # Override `lua.withPackages` to register the lux loader and expose lux trees
  # on LUA_PATH when a package in the environment was built with `buildLuxPackage`.
  override-lua = lua: lux-lua: let
    luaWithLuxLua = lua.override {
      packageOverrides = _final: _prev: {inherit lux-lua;};
    };
    withPackages = f: let
      packages = f luaWithLuxLua.pkgs;
      luxPackages = lib.filter (p: p ? luxPackage && p.luxPackage) packages;
      # The lux.loader searches the package.path for Lux trees.
      luxPathArgs =
        lib.concatMap (p: [
          "--suffix"
          "LUA_PATH"
          "';'"
          "'${p}/share/lux/${lua.luaversion}/?.lua'"
          "--suffix"
          "LUA_PATH"
          "';'"
          "'${p}/share/lux/${lua.luaversion}/?/init.lua'"
        ])
        luxPackages;
    in
      luaWithLuxLua.buildEnv.override {
        extraLibs = packages;
        makeWrapperArgs =
          lib.optionals (luxPackages != []) [
            "--set"
            "LUA_INIT"
            "\"require('lux').loader()\""
          ]
          ++ luxPathArgs;
      };
  in
    luaWithLuxLua.overrideAttrs (old: {
      passthru = (old.passthru or {}) // {inherit withPackages;};
    });

  # Convert a lux package (built with `--nvim`) into a Neovim plugin, exposing
  # the runtime files under site/pack/lux/start/<name> at the plugin root.
  toLuxNeovimPlugin = pkg: let
    nvimPkg = pkg.override {nvim = true;};
    luaVersionDir =
      if pkg.luaModule.pkgs.isLuaJIT
      then "jit"
      else pkg.luaModule.luaversion;
  in
    (final.symlinkJoin {
      name = "vimplugin-${pkg.pname}";
      paths = [nvimPkg];
      postBuild = ''
        for f in ${nvimPkg}/share/lux/${luaVersionDir}/site/pack/lux/start/*/*; do
          ln -sfn "$f" "$out/$(basename "$f")"
        done
      '';
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          vimPlugin = true;
          luxPackage = true;
          luxLuaVersion = luaVersionDir;
        };
    });

  # Override neovim to activate the lux loader and expose lux trees on
  # package.path when a plugin in the environment was built with lux.
  override-neovim = neovim-unwrapped: lux-lua:
    lib.makeOverridable (
      args: let
        luxPlugins = lib.filter (p: p ? luxPackage && p.luxPackage) (args.plugins or []);
        luxLuaPath =
          lib.concatMapStringsSep ";" (p: "${p}/share/lux/${p.luxLuaVersion}/?.lua;${p}/share/lux/${p.luxLuaVersion}/?/init.lua")
          luxPlugins;
      in
        final.wrapNeovimUnstable neovim-unwrapped (
          args
          // {
            extraLuaPackages = ps: ((args.extraLuaPackages or (_: [])) ps) ++ lib.optional (luxPlugins != []) lux-lua;
            luaRcContent =
              (args.luaRcContent or "")
              + lib.optionalString (luxPlugins != []) ''
                package.path = "${luxLuaPath};" .. package.path
                require('lux').loader()
              '';
          }
        )
    )
    {};
  cleanCargoSrc = craneLib.cleanCargoSource self;

  luxCargo = craneLib.crateNameFromCargoToml {
    src = self;
  };

  commonArgs = with final; {
    strictDeps = true;

    nativeBuildInputs = [
      pkg-config
      installShellFiles
    ];

    buildInputs = [
      openssl
      libgit2
      gnupg
      libgpg-error
      gpgme
    ];

    env = {
      # disable vendored packages
      LIBSSH2_SYS_USE_PKG_CONFIG = 1;
      LUX_SKIP_IMPURE_TESTS = 1;
    };
  };

  xtask-lua-deps = craneLib.buildDepsOnly (commonArgs
    // {
      pname = "xtask-lua";
      version = "0.1.0";
      src = cleanCargoSrc;

      env =
        commonArgs.env
        // {
          CARGO_PROFILE = "dev";
        };
      cargoExtraArgs = "-p xtask-lua --locked";

      buildInputs = commonArgs.buildInputs;
    });

  lux-deps = {release ? true}:
    craneLib.buildDepsOnly (commonArgs
      // {
        pname = "lux";
        version = "0.1.0";
        src = cleanCargoSrc;

        env =
          commonArgs.env
          // {
            CARGO_PROFILE =
              if release
              then "release"
              else "dev";
          };

        # perl is needed to build openssl-sys
        nativeBuildInputs = commonArgs.nativeBuildInputs ++ [final.perl];

        buildInputs = commonArgs.buildInputs;
      });

  individualCrateArgs = args @ {release ? true}:
    commonArgs
    // {
      env =
        commonArgs.env
        // {
          CARGO_PROFILE =
            if release
            then "release"
            else "dev";
        };
      src = cleanCargoSrc;
      cargoArtifacts = lux-deps args;
      # NOTE: We disable tests since we run them via cargo-nextest in a separate derivation
      doCheck = false;
    };

  mk-xtask-lua = luaFeature: let
    crateArgs = individualCrateArgs {release = false;};
  in
    craneLib.buildPackage (crateArgs
      // {
        pname = "xtask-${luaFeature}";
        inherit (luxCargo) version;

        buildInputs = crateArgs.buildInputs ++ [final.lua5_4];

        cargoArtifacts = xtask-lua-deps;
        cargoExtraArgs = "-p xtask-lua --features ${luaFeature}";

        meta.mainProgram = "xtask-lua";
      });

  mk-lux-lua = {
    release ? true,
    luaPkg,
    isLuaJIT,
  }: let
    luaMajorMinor = lib.take 2 (lib.splitVersion luaPkg.version);
    luxLuaVersionDir =
      if isLuaJIT
      then "jit"
      else lib.concatStringsSep "." luaMajorMinor;
    luaVersionDir =
      if isLuaJIT
      then "5.1"
      else lib.concatStringsSep "." luaMajorMinor;
    luaFeature =
      if isLuaJIT
      then "luajit"
      else "lua${lib.concatStringsSep "" luaMajorMinor}";
    dist-cmd =
      if release
      then "dist"
      else "dist-debug";
    crateArgs = individualCrateArgs {inherit release;};
  in
    luaPkg.pkgs.toLuaModule (
      craneLib.mkCargoDerivation (crateArgs
        // {
          pname = "lux-lua";
          inherit (luxCargo) version;

          # FIXME: This fails with permission denied on darwin
          buildPhaseCargoCommand = "xtask-lua ${dist-cmd}";
          nativeBuildInputs =
            crateArgs.nativeBuildInputs
            ++ [
              (mk-xtask-lua luaFeature)
            ];

          buildInputs = crateArgs.buildInputs ++ [luaPkg];

          # HACK: For some reason, linking via pkg-config fails on darwin
          env =
            (crateArgs.env or {})
            // final.lib.optionalAttrs final.stdenv.isDarwin {
              LUA_LIB = "${luaPkg}/lib";
              LUA_INCLUDE_DIR = "${luaPkg}/include";
              RUSTFLAGS = "-L ${luaPkg}/lib -llua";
            };

          installPhase = ''
            runHook preInstall
            install -D -v target/dist/share/lux-lua/${luxLuaVersionDir}/* -t $out/share/lux-lua/${luxLuaVersionDir}
            install -D -v target/dist/lib/pkgconfig/* -t $out/lib/pkgconfig
            mkdir -p $out/lib/lua
            ln -s $out/share/lux-lua/${luxLuaVersionDir} $out/lib/lua/${luaVersionDir}
            runHook postInstall
          '';
        })
    );

  mk-lux-cli = args: let
    crateArgs = individualCrateArgs args;
    cargoExtraArgs = "-p lux-cli --locked";
  in
    craneLib.buildPackage (crateArgs
      // {
        pname = "lux-cli";
        inherit (luxCargo) version;

        buildInputs =
          crateArgs.buildInputs;

        inherit cargoExtraArgs;

        postInstall = let
          lx = "${final.stdenv.hostPlatform.emulator final.buildPackages} $out/bin/lx";
        in ''
          ${lx} util man --target-dir="target/dist"
          ${lx} util completion --target-dir="target/dist"
          installManPage target/dist/*.1
          installShellCompletion target/dist/lx.{bash,fish} --zsh target/dist/_lx
        '';

        meta.mainProgram = "lx";
      });
  mk-lux-lsp = args: let
    crateArgs = individualCrateArgs args;
  in
    craneLib.buildPackage (crateArgs
      // {
        pname = "lux-lsp";
        inherit (luxCargo) version;

        cargoExtraArgs = "-p lux-lsp --locked";

        meta.mainProgram = "lx-lsp";
      });
in {
  inherit toLuxNeovimPlugin;

  fetchLuxDeps = {
    lua,
    lux-cli ? final.lux-cli,
  }:
    final.callPackage ./fetch-lux-deps.nix {
      inherit lua lux-cli;
    };

  importLuxLock = final.callPackage ./import-lux-lock.nix {};

  luxLoaderSetupHook = luaversion:
    final.makeSetupHook {
      name = "lux-loader-setup-hook";
      substitutions = {inherit luaversion;};
    }
    ./lux-loader-setup-hook.sh;

  lua5_1_lux = override-lua prev.lua5_1 final.lux-lua51;
  lua5_2_lux = override-lua prev.lua5_2 final.lux-lua52;
  lua5_3_lux = override-lua prev.lua5_3 final.lux-lua53;
  lua5_4_lux = override-lua prev.lua5_4 final.lux-lua54;
  lua5_5_lux = override-lua prev.lua5_5 final.lux-lua55;
  luajit_lux = override-lua prev.luajit final.lux-luajit;
  neovim-lux = lux-lua: override-neovim prev.neovim-unwrapped lux-lua;

  buildLuxPackage = {
    lua,
    lux-cli ? final.lux-cli,
  }:
    lib.makeOverridable (
      final.callPackage ./build-lux-package.nix {
        inherit lua lux-cli;
        fetchLuxDeps = final.fetchLuxDeps {inherit lua lux-cli;};
        importLuxLock = final.importLuxLock;
        luxLoaderSetupHook = final.luxLoaderSetupHook lua.luaversion;
      }
    );

  buildLuxRockspec = {
    lua,
    lux-cli ? final.lux-cli,
  }:
    lib.makeOverridable (
      lua.pkgs.callPackage ./build-lux-rockspec.nix {
        inherit lua lux-cli;
        fetchLuxDeps = final.fetchLuxDeps {inherit lua lux-cli;};
        luxLoaderSetupHook = final.luxLoaderSetupHook lua.luaversion;
      }
    );

  buildLuxApplication = {
    lua,
    lux-cli ? final.lux-cli,
  }:
    lib.makeOverridable (
      final.callPackage ./build-lux-application.nix {
        inherit lua;
        buildLuxPackage = final.buildLuxPackage {inherit lua lux-cli;};
      }
    );

  lux-cli = (mk-lux-cli {}).overrideAttrs {
    passthru.debug = mk-lux-cli {release = false;};
  };
  lux-lsp = (mk-lux-lsp {}).overrideAttrs {
    passthru.debug = mk-lux-lsp {release = false;};
  };
  lux-lua51 =
    (mk-lux-lua {
      luaPkg = prev.lua5_1;
      isLuaJIT = false;
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          debug = mk-lux-lua {
            luaPkg = prev.lua5_1;
            isLuaJIT = false;
            release = false;
          };
        };
    });
  lux-lua52 =
    (mk-lux-lua {
      luaPkg = prev.lua5_2;
      isLuaJIT = false;
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          debug = mk-lux-lua {
            luaPkg = prev.lua5_2;
            isLuaJIT = false;
            release = false;
          };
        };
    });
  lux-lua53 =
    (mk-lux-lua {
      luaPkg = prev.lua5_3;
      isLuaJIT = false;
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          debug = mk-lux-lua {
            luaPkg = prev.lua5_3;
            isLuaJIT = false;
            release = false;
          };
        };
    });
  lux-lua54 =
    (mk-lux-lua {
      luaPkg = prev.lua5_4;
      isLuaJIT = false;
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          debug = mk-lux-lua {
            luaPkg = prev.lua5_4;
            isLuaJIT = false;
            release = false;
          };
        };
    });
  lux-lua55 =
    (mk-lux-lua {
      luaPkg = prev.lua5_5;
      isLuaJIT = false;
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          debug = mk-lux-lua {
            luaPkg = prev.lua5_5;
            isLuaJIT = false;
            release = false;
          };
        };
    });
  lux-luajit =
    (mk-lux-lua {
      luaPkg = prev.luajit;
      isLuaJIT = true;
    }).overrideAttrs (old: {
      passthru =
        (old.passthru or {})
        // {
          debug = mk-lux-lua {
            luaPkg = prev.luajit;
            isLuaJIT = true;
            release = false;
          };
        };
    });
  lux-workspace-hack = craneLib.mkCargoDerivation {
    src = cleanCargoSrc;
    pname = "lux-workspace-hack";
    version = "0.1.0";
    cargoArtifacts = null;
    doInstallCargoArtifacts = false;

    buildPhaseCargoCommand = ''
      cargo hakari generate --diff
      cargo hakari manage-deps --dry-run
      cargo hakari verify
    '';

    nativeBuildInputs = with final; [
      cargo-hakari
    ];
  };

  lux-nextest = craneLib.cargoNextest (commonArgs
    // {
      pname = "lux-tests";
      inherit (luxCargo) version;
      src = self;

      buildInputs =
        commonArgs.buildInputs
        ++ [
          # Must be the same as the nativeCheckInputs lua
          final.lua5_4
        ];

      env =
        commonArgs.env
        // {
          CARGO_PROFILE = "test";
        };

      nativeCheckInputs = with final; [
        # Must be the same as the buildInputs lua, otherwise pkg-config won't find it
        lua5_4
        cacert
        cargo-nextest
        zlib # used for checking external dependencies
        nix # we use nix-hash in tests
      ];

      cargoArtifacts = lux-deps {release = false;};
      partitions = 1;
      partitionType = "count";
      cargoNextestExtraArgs = "--no-fail-fast --lib"; # Disable integration tests
      cargoNextestPartitionsExtraArgs = "--no-tests=pass";
    });

  lux-nextest-lua = craneLib.cargoNextest (commonArgs
    // {
      pname = "lux-lua";
      version = "0.1.0";
      src = self;
      cargoExtraArgs = "-p lux-lua --locked --features test";
      buildInputs =
        commonArgs.buildInputs
        ++ [
          # Must be the same as the nativeCheckInputs lua
          final.lua5_1
        ];

      env =
        commonArgs.env
        // {
          CARGO_PROFILE = "test";
        };

      nativeCheckInputs = with final; [
        cacert
        cargo-nextest
        zlib # used for checking external dependencies
        # Must be the same as the buildInputs lua, otherwise pkg-config won't find it
        lua5_1
        nix # we use nix-hash in tests
      ];

      cargoArtifacts = lux-deps {release = false;};
      partitions = 1;
      partitionType = "count";
      cargoNextestExtraArgs = "--no-fail-fast --lib"; # Disable integration tests
      cargoNextestPartitionsExtraArgs = "--no-tests=pass";
    });

  lux-clippy = craneLib.cargoClippy (commonArgs
    // {
      env =
        commonArgs.env
        // {
          CARGO_PROFILE = "dev";
        };
      pname = "lux-clippy";
      inherit (luxCargo) version;
      src = cleanCargoSrc;
      buildInputs = commonArgs.buildInputs ++ [final.lua5_4];
      cargoArtifacts = lux-deps {release = false;};
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
    });
}
