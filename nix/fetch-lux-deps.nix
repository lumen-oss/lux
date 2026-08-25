{
  lib,
  stdenvNoCC,
  lux-cli,
  git,
  cacert,
}:
lib.extendMkDerivation {
  constructDrv = stdenvNoCC.mkDerivation;

  excludeDrvArgNames = [
    "hash"
    "luxLock"
    "luxRoot"
  ];

  extendDrvArgs = finalAttrs: {
    pname ? "lux",
    version ? "0",
    hash,
    src,
    luxLock ? null,
    luxRoot ? null,
    nativeBuildInputs ? [],
    ...
  }: {
    name = "${pname}-${version}-vendor-deps";

    inherit src;

    nativeBuildInputs =
      [
        lux-cli
        git
        cacert
      ]
      ++ nativeBuildInputs;

    impureEnvVars = lib.fetchers.proxyImpureEnvVars;

    env = {
      SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
      GIT_SSL_CAINFO = "${cacert}/etc/ssl/certs/ca-bundle.crt";
    };

    buildPhase = ''
      runHook preBuild

      ${lib.optionalString (luxRoot != null) "cd ${luxRoot}"}

      ${lib.optionalString (luxLock != null) "cp ${luxLock} lux.lock"}

      lx vendor "$out" --no-delete

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
