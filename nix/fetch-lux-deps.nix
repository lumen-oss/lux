{
  lib,
  stdenvNoCC,
  lux-cli,
  git,
  cacert,
  lua,
  cargo,
  writableTmpDirAsHomeHook,
}:
lib.extendMkDerivation {
  constructDrv = stdenvNoCC.mkDerivation;

  excludeDrvArgNames = [
    "hash"
    "luxLock"
    "luxRoot"
    "rockspecFilename"
    "knownRockspec"
    "luaVersion"
    "nvim"
    "rustSupport"
  ];

  extendDrvArgs = finalAttrs: {
    pname ? "lux",
    version ? "0",
    hash,
    src,
    luxLock ? null,
    luxRoot ? null,
    rockspecFilename ? null,
    knownRockspec ? null,
    luaVersion ? null,
    nvim ? false,
    rustSupport ? false,
    nativeBuildInputs ? [],
    ...
  }: let
    luaVersionFlag =
      if nvim
      then "--nvim"
      else lib.optionalString (luaVersion != null) "--lua-version '${luaVersion}'";
    vendorCmd =
      if rockspecFilename != null || knownRockspec != null
      then ''
        ${
          if knownRockspec != null
          then "cp ${knownRockspec} ./pkg.rockspec"
          else "cp \"$src/${rockspecFilename}\" ./pkg.rockspec"
        }

        lx ${luaVersionFlag} vendor "$out" --rockspec ./pkg.rockspec --no-delete
      ''
      else ''
        lx ${luaVersionFlag} vendor "$out" --no-delete
      '';
  in {
    name = "${pname}-${version}-vendor-deps";

    inherit src;

    nativeBuildInputs =
      [
        lux-cli
        lua
        git
        cacert
      ]
      ++ lib.optional rustSupport [
        cargo
        writableTmpDirAsHomeHook
      ]
      ++ nativeBuildInputs;

    impureEnvVars = lib.fetchers.proxyImpureEnvVars;

    env = {
      SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
      GIT_SSL_CAINFO = "${cacert}/etc/ssl/certs/ca-bundle.crt";
    };

    buildPhase = ''
      runHook preBuild

      mkdir -p $out

      ${lib.optionalString (luxRoot != null) "cd ${luxRoot}"}

      ${lib.optionalString (luxLock != null) "cp ${luxLock} lux.lock"}

      ${vendorCmd}

      runHook postBuild
    '';

    strictDeps = true;

    dontConfigure = true;
    dontInstall = true;
    dontFixup = true;

    outputHash = hash;
    outputHashAlgo =
      if hash == ""
      then "sha256"
      else null;
    outputHashMode = "recursive";
  };
}
