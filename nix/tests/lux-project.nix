{pkgs}: let
  src = pkgs.runCommandLocal "toml-edit-wrapper-src" {} ''
        mkdir -p "$out/src"
        cat > "$out/lux.toml" <<'TOML'
    package = "toml-edit-wrapper"
    version = "0.1.0"
    lua = ">=5.1"

    [description]
    summary = "test"
    maintainer = "lumen-oss"
    license = "MIT"

    [dependencies]
    toml-edit = "0.7.0-1"

    [build]
    type = "builtin"

    [build.modules]
    toml_edit_wrapper = "src/wrapper.lua"
    TOML
        cat > "$out/src/wrapper.lua" <<'LUA'
    local toml_edit = require("toml_edit")
    return {
      parse = function(s) return toml_edit.parse(s) end,
    }
    LUA
  '';

  lua = pkgs.lua5_1.override {
    packageOverrides = _final: _prev: {"lux-lua" = pkgs.lux-lua51.debug;};
  };

  buildLuxPackage = pkgs.buildLuxPackage {
    inherit lua;
    lux-cli = pkgs.lux-cli.debug;
  };

  tomlEditWrapper = buildLuxPackage {
    pname = "toml-edit-wrapper";
    version = "0.1.0";
    inherit src;
    luxHash = "sha256-k68Gh9R9Fk7PfT4szSQgopzZO49qllby3Sl+AExMnng=";
    rustSupport = true;
  };
in {
  test = pkgs.runCommandLocal "lux-project-toml-edit-test" {} ''
    set -euo pipefail
    ${pkgs.lua5_1.withPackages (_: [tomlEditWrapper])}/bin/lua -e '
      local wrapper = require("toml_edit_wrapper")
      assert(wrapper ~= nil)
      local toml_edit = require("toml_edit")
      assert(toml_edit ~= nil)
      print("toml-edit loaded via lux project dependency")
    '
    touch "$out"
  '';
}
