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
      LIBGIT2_NO_VENDOR = 1;
      LIBSSH2_SYS_USE_PKG_CONFIG = 1;
      LUX_SKIP_IMPURE_TESTS = 1;
    };
  };

  lux-deps = craneLib.buildDepsOnly (commonArgs
    // {
      pname = "lux";
      version = "0.1.0";
      src = cleanCargoSrc;

      # perl is needed to build openssl-sys
      nativeBuildInputs = commonArgs.nativeBuildInputs ++ [final.perl];

      buildInputs = commonArgs.buildInputs ++ [final.lua5_4];
    });

  individualCrateArgs =
    commonArgs
    // {
      src = cleanCargoSrc;
      cargoArtifacts = lux-deps;
      # NOTE: We disable tests since we run them via cargo-nextest in a separate derivation
      doCheck = false;
    };

  mk-xtask-lua = luaFeature:
    craneLib.buildPackage (individualCrateArgs
      // {
        pname = "xtask-${luaFeature}";
        inherit (luxCargo) version;

        buildInputs = individualCrateArgs.buildInputs ++ [final.lua5_4];

        cargoExtraArgs = "-p xtask-lua --features ${luaFeature}";

        meta.mainProgram = "xtask-lua";
      });

  mk-lux-lua = {
    buildType ? "release",
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
  in
    craneLib.buildPackage (individualCrateArgs
      // {
        pname = "lux-lua";
        inherit (luxCargo) version;

        # FIXME: This fails with permission denied on darwin
        cargoBuildCommand = "xtask-lua dist";
        nativeBuildInputs =
          individualCrateArgs.nativeBuildInputs
          ++ [
            (mk-xtask-lua luaFeature)
          ];

        buildInputs = individualCrateArgs.buildInputs ++ [luaPkg];

        # HACK: For some reason, linking via pkg-config fails on darwin
        env =
          (individualCrateArgs.env or {})
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

  xtask = craneLib.buildPackage (individualCrateArgs
    // {
      pname = "xtask";
      inherit (luxCargo) version;

      buildInputs = individualCrateArgs.buildInputs ++ [final.lua5_4];

      cargoExtraArgs = "-p xtask";

      meta.mainProgram = "xtask";
    });

  # can't seem to override the buildType with override or overrideAttrs :(
  mk-lux-cli = {buildType ? "release"}:
    craneLib.buildPackage (individualCrateArgs
      // {
        pname = "lux-cli";
        inherit (luxCargo) version;

        nativeBuildInputs =
          individualCrateArgs.nativeBuildInputs
          ++ [
            xtask
          ];

        buildInputs =
          individualCrateArgs.buildInputs
          ++ [
            final.luau
          ];

        cargoBuildCommand = "cargo build --profile ${buildType}";
        cargoExtraArgs = "-p lux-cli --locked";

        postBuild =
          if final.stdenv.isDarwin
          # For some reason, xtask errors with "permission denied" on darwin
          then ""
          else ''
            xtask dist-man
            xtask dist-completions
          '';

        postInstall =
          if final.stdenv.isDarwin
          then ""
          else ''
            installManPage target/dist/lx.1
            installShellCompletion target/dist/lx.{bash,fish} --zsh target/dist/_lx
          '';

        meta.mainProgram = "lx";
      });
in {
  inherit xtask;
  lux-cli = mk-lux-cli {};
  lux-cli-debug = mk-lux-cli {buildType = "debug";};
  lux-lua51 = mk-lux-lua {
    luaPkg = final.lua5_1;
    isLuaJIT = false;
  };
  lux-lua52 = mk-lux-lua {
    luaPkg = final.lua5_2;
    isLuaJIT = false;
  };
  lux-lua53 = mk-lux-lua {
    luaPkg = final.lua5_3;
    isLuaJIT = false;
  };
  lux-lua54 = mk-lux-lua {
    luaPkg = final.lua5_4;
    isLuaJIT = false;
  };
  lux-lua55 = mk-lux-lua {
    luaPkg = final.lua5_5;
    isLuaJIT = false;
  };
  lux-luajit = mk-lux-lua {
    luaPkg = final.luajit;
    isLuaJIT = true;
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
          # make sure this is the same as the nativeCheckInputs lua
          final.lua5_4
        ];

      nativeCheckInputs = with final; [
        # make sure this is the same as the buildInputs lua, otherwise pkg-config won't find it
        lua5_4
        cacert
        cargo-nextest
        zlib # used for checking external dependencies
        nix # we use nix-hash in tests
      ];

      cargoArtifacts = lux-deps;
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
      buildInputs = commonArgs.buildInputs;

      nativeCheckInputs = with final; [
        cacert
        cargo-nextest
        zlib # used for checking external dependencies
        lua5_1
        nix # we use nix-hash in tests
      ];

      cargoArtifacts = lux-deps;
      partitions = 1;
      partitionType = "count";
      cargoNextestExtraArgs = "--no-fail-fast --lib"; # Disable integration tests
      cargoNextestPartitionsExtraArgs = "--no-tests=pass";
    });

  lux-taplo = craneLib.craneLib.taploFmt {
    inherit (luxCargo) pname version;
    src = lib.fileset.toSource {
      root = ../.;
      # Don't format the contents of the autogenerated workspace hack crate
      fileset = lib.fileset.difference ../. ../lux-workspace-hack;
    };
  };

  lux-clippy = craneLib.cargoClippy (commonArgs
    // {
      pname = "lux-clippy";
      inherit (luxCargo) version;
      src = cleanCargoSrc;
      buildInputs = commonArgs.buildInputs ++ [final.lua5_4];
      cargoArtifacts = lux-deps;
      cargoClippyExtraArgs = "--all-targets -- --deny warnings";
    });
}
