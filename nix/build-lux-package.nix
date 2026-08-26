{
  lib,
  stdenv,
  fetchLuxDeps,
  importLuxLock,
  lux-cli,
  luxLoaderSetupHook,
  pkg-config,
  lua,
  cargo,
  rustc,
  writableTmpDirAsHomeHook,
}:
lib.extendMkDerivation {
  constructDrv = stdenv.mkDerivation;

  excludeDrvArgNames = [
    "luxDeps"
    "luxHash"
    "luxLock"
    "luxRoot"
    "luxVendorDir"
    "rustSupport"
  ];

  transformDrv = drv:
    (lua.pkgs.toLuaModule drv).overrideAttrs (old: {
      passthru = (old.passthru or {}) // {luxPackage = true;};
    });

  extendDrvArgs = finalAttrs: {
    src,
    luxLock ? null,
    luxHash ? null,
    luxDeps ? null,
    luxVendorDir ? null,
    luxRoot ? null,
    rustSupport ? false,
    buildAndTestSubdir ? null,
    nvim ? false,
    nativeBuildInputs ? [],
    propagatedBuildInputs ? [],
    postUnpack ? "",
    ...
  }: let
    luxLuaVersion =
      if lua.pkgs.isLuaJIT
      then "jit"
      else lua.luaversion;
    luaVersionFlag =
      if nvim
      then "--nvim"
      else "--lua-version \"${luxLuaVersion}\"";

    deps =
      if luxVendorDir != null
      then luxVendorDir
      else if luxDeps != null
      then luxDeps
      else if luxLock != null
      then importLuxLock {lockFile = luxLock;}
      else if luxHash != null
      then
        fetchLuxDeps {
          inherit src luxRoot rustSupport nvim;
          pname = finalAttrs.pname;
          version = finalAttrs.version;
          luaVersion = lua.luaversion;
          hash = luxHash;
        }
      else throw "buildLuxPackage requires either (sorted by precedence) `luxVendorDir`, `luxDeps`, `luxLock` or `luxHash`";

    rootSubdir = lib.optionalString (luxRoot != null) "${luxRoot}/";

    buildSubdir = lib.optionalString (buildAndTestSubdir != null) "${buildAndTestSubdir}/";

    lockFile =
      if luxLock != null
      then luxLock
      else if !lib.isDerivation src && lib.pathExists "${src}/${rootSubdir}lux.lock"
      then "${src}/${rootSubdir}lux.lock"
      else null;

    expectedRocks =
      if lockFile == null
      then []
      else let
        lock = builtins.fromJSON (lib.readFile lockFile);
      in
        lib.concatMap (
          section:
            lib.mapAttrsToList (_id: rock: "${rock.name}@${rock.version}") (
              lock.${section}.rocks or {}
            )
        ) [
          "dependencies"
          "test_dependencies"
          "build_dependencies"
        ];
  in {
    name = "lua${lua.luaversion}-${finalAttrs.pname}-${finalAttrs.version}";

    __structuredAttrs = true;
    strictDeps = true;

    nativeBuildInputs =
      [
        lux-cli
        pkg-config
        lua
      ]
      ++ lib.optionals rustSupport [
        cargo
        rustc
        writableTmpDirAsHomeHook
      ]
      ++ nativeBuildInputs;

    propagatedBuildInputs =
      [
        lua
        lua.pkgs.lux-lua
        luxLoaderSetupHook
      ]
      ++ propagatedBuildInputs;

    postUnpack =
      postUnpack
      + lib.optionalString (lockFile != null) ''
        for rock in ${lib.escapeShellArgs expectedRocks}; do
          if [ ! -e "${deps}/$rock" ] && [ ! -e "${deps}/$rock.rock" ]; then
            echo "ERROR: missing vendored rock: $rock" >&2
            echo "The vendored dependencies are out of date (stale luxHash, luxDeps, luxLock or luxVendorDir)." >&2
            echo >&2
            echo "To fix the issue:" >&2
            echo '1. Set luxHash to an empty string: `luxHash = "";` (or re-vendor luxDeps/luxLock/luxVendorDir)' >&2
            echo '2. Build the derivation and wait for it to fail with a hash mismatch' >&2
            echo '3. Copy the "got: sha256-..." value back into the luxHash field' >&2
            echo >&2
            echo 'Note: If you are trying to override luxHash, set `luxDeps = fetchLuxDeps ...` instead.' >&2
            echo >&2
            exit 1
          fi
        done
      '';

    buildPhase = ''
      runHook preBuild

      if (( ''${NIX_DEBUG:-0} >= 1 )); then
        export RUST_LOG="trace"
      fi

      ${lib.optionalString (buildAndTestSubdir != null) "pushd ${buildAndTestSubdir}"}

      ${lib.optionalString (luxLock != null) "install -m 644 ${luxLock} lux.lock"}

      lx --profile release \
         ${luaVersionFlag} \
         --vendor-dir "${deps}" \
         build

      ${lib.optionalString (buildAndTestSubdir != null) "popd"}

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p "$out/share/lux"
      cp -rT "${buildSubdir}.lux" "$out/share/lux"

      runHook postInstall
    '';

    checkPhase = ''
      runHook preCheck

      ${lib.optionalString (buildAndTestSubdir != null) "pushd ${buildAndTestSubdir}"}

      lx --profile release \
         ${luaVersionFlag} \
         --vendor-dir "${deps}" \
         --tree "$TMPDIR/lux-test" \
         test

      ${lib.optionalString (buildAndTestSubdir != null) "popd"}

      runHook postCheck
    '';
  };
}
