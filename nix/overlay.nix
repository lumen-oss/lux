{
  self,
  crane,
}: final: prev: let
  lib = final.lib;
  craneLib = crane.mkLib prev;

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
    luaVersionDir =
      if isLuaJIT
      then "jit"
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
          install -D -v target/dist/share/lux-lua/${luaVersionDir}/* -t $out/share/lux-lua/${luaVersionDir}
          install -D -v target/dist/lib/pkgconfig/* -t $out/lib/pkgconfig
          runHook postInstall
        '';
      });

  mk-lux-cli = args @ {release ? true}: let
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
  mk-lux-lsp = args @ {release ? true}: let
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
  lux-cli = (mk-lux-cli {}).overrideAttrs {
    passthru.debug = mk-lux-cli {release = false;};
  };
  lux-lsp = (mk-lux-lsp {}).overrideAttrs {
    passthru.debug = mk-lux-lsp {release = false;};
  };
  lux-lua51 =
    (mk-lux-lua {
      luaPkg = final.lua5_1;
      isLuaJIT = false;
    }).overrideAttrs {
      passthru.debug = mk-lux-lua {
        luaPkg = final.lua5_1;
        isLuaJIT = false;
        release = false;
      };
    };
  lux-lua52 =
    (mk-lux-lua {
      luaPkg = final.lua5_2;
      isLuaJIT = false;
    }).overrideAttrs {
      passthru.debug = mk-lux-lua {
        luaPkg = final.lua5_2;
        isLuaJIT = false;
        release = false;
      };
    };
  lux-lua53 =
    (mk-lux-lua {
      luaPkg = final.lua5_3;
      isLuaJIT = false;
    }).overrideAttrs {
      passthru.debug = mk-lux-lua {
        luaPkg = final.lua5_3;
        isLuaJIT = false;
        release = false;
      };
    };
  lux-lua54 =
    (mk-lux-lua {
      luaPkg = final.lua5_4;
      isLuaJIT = false;
    }).overrideAttrs {
      passthru.debug = mk-lux-lua {
        luaPkg = final.lua5_4;
        isLuaJIT = false;
        release = false;
      };
    };
  lux-lua55 =
    (mk-lux-lua {
      luaPkg = final.lua5_5;
      isLuaJIT = false;
    }).overrideAttrs {
      passthru.debug = mk-lux-lua {
        luaPkg = final.lua5_5;
        isLuaJIT = false;
        release = false;
      };
    };
  lux-luajit =
    (mk-lux-lua {
      luaPkg = final.luajit;
      isLuaJIT = true;
    }).overrideAttrs {
      passthru.debug = mk-lux-lua {
        luaPkg = final.luajit;
        isLuaJIT = true;
        release = false;
      };
    };

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
