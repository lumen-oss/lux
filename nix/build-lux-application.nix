{
  lib,
  buildLuxPackage,
  makeWrapper,
  lua,
}: {
  runCommand ? null,
  runArgs ? [],
  ...
} @ args: let
  interpreter =
    if runCommand == null
    then "${lua}/bin/lua"
    else runCommand;
in
  buildLuxPackage (
    removeAttrs args [
      "runArgs"
      "runCommand"
    ]
    // {
      nativeBuildInputs = args.nativeBuildInputs or [] ++ [makeWrapper];

      postInstall =
        (args.postInstall or "")
        + ''
          cp -r "$src/." "$out/"
          mkdir -p "$out/bin"

          LUA_PATH="$(lx --lua-version "${lua.luaversion}" --tree "$out/share/lux" path lua)"
          LUA_CPATH="$(lx --lua-version "${lua.luaversion}" --tree "$out/share/lux" path c)"
          LUA_INIT="$(lx --lua-version "${lua.luaversion}" --tree "$out/share/lux" path init)"

          makeWrapper "${interpreter}" "$out/bin/${args.pname}" \
            --chdir "$out" \
            --set LUA_INIT "$LUA_INIT" \
            --set LUA_PATH "$LUA_PATH" \
            --set LUA_CPATH "$LUA_CPATH" \
            --add-flags ${lib.escapeShellArg (lib.escapeShellArgs runArgs)}
        '';
    }
  )
