{pkgs}: let
  src = pkgs.fetchFromGitHub {
    owner = "NTBBloodbath";
    repo = "fallo";
    rev = "c2efe9ad31c97265afb696b62e90312a6b944dc2";
    hash = "sha256-imyKEDvbn9EWJ8Cm54v5sUKZV2fZuBTT3du+1ipKgW4=";
  };

  lua = pkgs.lua5_1_lux.override {
    packageOverrides = _final: _prev: {lux-lua = pkgs.lux-lua51.debug;};
  };

  buildLuxApplication = pkgs.buildLuxApplication {
    inherit lua;
    lux-cli = pkgs.lux-cli.debug;
  };

  buildLuxPackage = pkgs.buildLuxPackage {
    inherit lua;
    lux-cli = pkgs.lux-cli.debug;
  };

  mkTest = name: deps: let
    fallo = buildLuxApplication (
      {
        pname = "fallo";
        version = "0-unstable-2026-03-25";
        inherit src;
        doCheck = true;
        postInstall = ''
          cat > "$out/test-entrypoint.lua" <<'LUA'
          local Result = require("fallo")
          local ok = Result.ok(42)
          assert(ok.value == 42)
          print("fallo loaded via lux.loader")
          LUA
        '';
        runArgs = ["test-entrypoint.lua"];
      }
      // deps
    );
  in
    pkgs.runCommandLocal name {} ''
      set -euo pipefail
      ${fallo}/bin/fallo | grep -q "fallo loaded via lux.loader"
      touch "$out"
    '';
in {
  fetchLuxDeps = mkTest "build-lux-helpers-test" {
    luxHash = "sha256-DpkDMK6AV7Iy6YFbKbx+EEkDeGaAFbszNkLPhdVr+qY=";
  };

  importLuxLock = mkTest "build-lux-helpers-import-lock-test" {
    luxDeps = pkgs.importLuxLock {
      lockFileContents = builtins.readFile "${src}/lux.lock";
    };
  };

  luxLock = mkTest "build-lux-helpers-lux-lock-test" {
    luxLock = "${src}/lux.lock";
  };

  luxVendorDir = mkTest "build-lux-helpers-lux-vendor-dir-test" {
    luxVendorDir = pkgs.importLuxLock {
      lockFileContents = builtins.readFile "${src}/lux.lock";
    };
  };

  withPackages = let
    fallo = buildLuxPackage {
      pname = "fallo";
      version = "0-unstable-2026-03-25";
      inherit src;
      luxHash = "sha256-DpkDMK6AV7Iy6YFbKbx+EEkDeGaAFbszNkLPhdVr+qY=";
    };
  in
    pkgs.runCommandLocal "build-lux-helpers-with-packages-test" {} ''
      set -euo pipefail
      ${lua.withPackages (_: [fallo])}/bin/lua -e '
        local Result = require("fallo")
        local ok = Result.ok(42)
        assert(ok.value == 42)
        local cjson = require("cjson")
        assert(cjson.encode({ok = true}) == "{\"ok\":true}")
        print("fallo loaded via withPackages")
      '
      touch "$out"
    '';
}
