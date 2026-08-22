{
  lib,
  runCommand,
  fetchurl,
  fetchgit,
}: {
  lockFile ? null,
  lockFileContents ? null,
}:
assert lib.assertMsg ((lockFile == null) != (lockFileContents == null)) ''
  importLuxLock: Either `lockFile` or `lockFileContents` must be set.
''; let
  lockContents =
    if lockFile != null
    then builtins.readFile lockFile
    else lockFileContents;

  lock = builtins.fromJSON lockContents;

  lockSections = [
    "dependencies"
    "test_dependencies"
    "build_dependencies"
  ];

  rocks = lib.attrValues (
    lib.listToAttrs (
      map (rock: {
        name = "${rock.name}-${rock.version}";
        value = rock;
      }) (
        lib.flatten (
          map (section: lib.attrValues (lock.${section}.rocks or {})) lockSections
        )
      )
    )
  );

  rockspecServer = rock:
    lib.concatStringsSep "+" (lib.tail (lib.splitString "+" (rock.source or "")));

  fetchRockspec = rock:
    fetchurl {
      url = "${rockspecServer rock}${rock.name}-${rock.version}.rockspec";
      sha256 = rock.hashes.rockspec;
    };

  fetchSource = rock: let
    url = rock.source_url or {};
  in
    if url.type or "" == "git"
    then
      fetchgit {
        url = url.url;
        rev = url.ref;
        sha256 = rock.hashes.source;
        fetchSubmodules = url.submodules or false;
      }
    else if url.type or "" == "url"
    then
      fetchurl {
        url = url.url;
        sha256 = rock.hashes.source;
      }
    else throw "importLuxLock: unsupported `source_url.type` `${url.type or "?"}`";

  installRock = rock: ''
    ln -s ${fetchSource rock} "$out/${rock.name}@${rock.version}"
    ln -s ${fetchRockspec rock} "$out/${rock.name}-${rock.version}.rockspec"
  '';
in
  runCommand "lux-vendor-dir" {} ''
    mkdir -p "$out"
    ${lib.concatStrings (map installRock rocks)}
  ''
