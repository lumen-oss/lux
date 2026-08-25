{
  lib,
  stdenv,
  lua,
  luaLib,
  lux-cli,
  luxLoaderSetupHook,
  fetchLuxDeps,
  pkg-config,
  cargo,
  rustc,
  writeTextFile,
  writableTmpDirAsHomeHook,
}:
lib.extendMkDerivation {
  constructDrv = stdenv.mkDerivation;

  excludeDrvArgNames = [
    "knownRockspec"
    "rockspecVersion"
    "nvim"
    "luxHash"
    "luxDeps"
    "luxVendorDir"
    "rustSupport"
  ];

  transformDrv = drv:
    (lua.pkgs.toLuaModule drv).overrideAttrs (old: {
      passthru = (old.passthru or {}) // {luxPackage = true;};
    });

  extendDrvArgs = finalAttrs: {
    src,
    knownRockspec ? null,
    rockspecVersion ? finalAttrs.version,
    nvim ? false,
    luxHash ? null,
    luxDeps ? null,
    luxVendorDir ? null,
    rustSupport ? false,
    nativeBuildInputs ? [],
    propagatedBuildInputs ? [],
    ...
  }: let
    inherit (finalAttrs) pname version;

    luaVersionDir = lua.luaversion;
    luaVersionFlag =
      if nvim
      then "--nvim"
      else "--lua-version '${luaVersionDir}'";

    luaDeps = lib.filter (drv: drv ? luaModule) propagatedBuildInputs;
    luarocksDeps =
      lib.filter (
        drv:
          if drv ? luxPackage
          then lib.warn "buildLuxRockspec: lux-built dependency ${drv.pname} is not vendored; the build will fail if the rockspec requires it" false
          else true
      )
      luaDeps;

    luarocksConfig = luaLib.generateLuarocksConfig {
      local_cache = "";
      requiredLuaRocks = luarocksDeps;
    };

    luarocksConfigFile = writeTextFile {
      name = finalAttrs.pname + "-luarocks-config.lua";
      text = lib.generators.toLua {asBindings = true;} luarocksConfig;
    };

    rockspecFilename =
      if knownRockspec != null
      then knownRockspec
      else "${pname}-${rockspecVersion}.rockspec";

    vendoredDeps =
      if luxVendorDir != null
      then luxVendorDir
      else if luxDeps != null
      then luxDeps
      else if luxHash != null
      then
        fetchLuxDeps
        {
          inherit src pname rustSupport knownRockspec rockspecFilename nvim;
          version = rockspecVersion;
          luaVersion = luaVersionDir;
          hash = luxHash;
        }
      else null;
  in {
    name = "lua${luaVersionDir}-${pname}-${version}";
    inherit src;

    __structuredAttrs = true;
    strictDeps = true;
    env = {
      LUAROCKS_CONFIG = luarocksConfigFile;
    };

    nativeBuildInputs =
      [
        lux-cli
        pkg-config
        (lua.withPackages (ps: lib.optional (luarocksDeps != []) ps.luarocks))
      ]
      ++ lib.optionals rustSupport
      [
        cargo
        rustc
        writableTmpDirAsHomeHook
      ]
      ++ nativeBuildInputs;

    propagatedBuildInputs =
      [lua lua.pkgs.lux-lua luxLoaderSetupHook] ++ propagatedBuildInputs;

    buildPhase = ''
      runHook preBuild

      cp ${rockspecFilename} ./pkg.rockspec

      # Provide the package's own source through a vendor dir, so lux fetches
      # it locally instead of from the rockspec's remote URL.
      mkdir -p ./vendor

      ${lib.concatStringsSep "\n" (map (dep: ''
          luarocks pack ${dep.pname} ${dep.version}
          ROCKSPEC_FILENAME=${dep.pname}-${dep.version}.rockspec
          ROCKSPEC=${dep}/${dep.pname}-${dep.version}-rocks/${dep.pname}/${dep.version}/$ROCKSPEC_FILENAME
          cp $ROCKSPEC ./vendor/$ROCKSPEC_FILENAME
          cp ${dep.pname}-${dep.version}.*.rock ./vendor/${dep.pname}@${dep.version}.rock
        '')
        luarocksDeps)}

      ${lib.optionalString (vendoredDeps != null) "cp -r ${vendoredDeps}/. ./vendor/"}

      cp -r "$src" "./vendor/${pname}@${rockspecVersion}"
      chmod -R u+w ./vendor

      lx --profile release \
         ${luaVersionFlag} \
         --vendor-dir "$PWD/vendor" \
         --tree ./.lux \
         --no-wrap-bin \
         install-rockspec ./pkg.rockspec

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p "$out/share/lux"
      cp -rT .lux "$out/share/lux"

      runHook postInstall
    '';
  };
}
