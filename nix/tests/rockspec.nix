{pkgs}: let
  lua = pkgs.lua5_1_lux.override {
    packageOverrides = _final: _prev: {lux-lua = pkgs.lux-lua51.debug;};
  };

  buildLuxRockspec = pkgs.buildLuxRockspec {
    inherit lua;
    lux-cli = pkgs.lux-cli.debug;
  };

  lua-cjson = buildLuxRockspec {
    pname = "lua-cjson";
    version = "2.1.0.10-1";
    src = pkgs.fetchFromGitHub {
      owner = "openresty";
      repo = "lua-cjson";
      tag = "2.1.0.10";
      hash = "sha256-/SeQro0FaJn91bAGjsVIin+mJF89VUm/G0KyJkV9Qps=";
    };
    knownRockspec = pkgs.fetchurl {
      url = "mirror://luarocks/lua-cjson-2.1.0.10-1.rockspec";
      hash = "sha256-r+WGLILja827kVC0giYvUXOmvQeQhRb9bJN0cXA+Vxc=";
    };
  };

  luassert = buildLuxRockspec {
    pname = "luassert";
    version = "1.9.0-1";
    src = pkgs.fetchFromGitHub {
      owner = "lunarmodules";
      repo = "luassert";
      tag = "v1.9.0";
      hash = "sha256-jjdB95Vr5iVsh5T7E84WwZMW6/5H2k2R/ny2VBs2l3I=";
    };
    knownRockspec = pkgs.fetchurl {
      url = "mirror://luarocks/luassert-1.9.0-1.rockspec";
      hash = "sha256-rTPvF/GK/jMnH/q4wbwTCGBFELWh+JcvHeOCFAbIf64=";
    };
    propagatedBuildInputs = [pkgs.lua5_1.pkgs.say];
  };

  mkRequireTest = name: rock: mod:
    pkgs.runCommandLocal name {} ''
      set -euo pipefail
      ${lua.withPackages (_: [rock])}/bin/lua -e "assert(require('${mod}'))"
      touch "$out"
    '';
in {
  cjson = mkRequireTest "rockspec-lua-cjson-test" lua-cjson "cjson";
  luassert = mkRequireTest "rockspec-luassert-test" luassert "luassert";
}
