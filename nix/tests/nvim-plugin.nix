{pkgs}: let
  lua = pkgs.luajit.override {
    packageOverrides = _final: _prev: {"lux-lua" = pkgs.lux-luajit.debug;};
  };

  vendored-dep = pkgs.runCommand "foo-nvim-vendor" {} ''
    mkdir -p $out
    cat > $out/bar-1.0.0-1.rockspec <<'EOF'
    package = "bar"
    version = "1.0.0-1"
    source = { url = "https://example.com/bar" }
    build = { type = "builtin", modules = { bar = "lua/bar.lua" } }
    EOF
    mkdir -p $out/bar@1.0.0-1/lua
    cat > $out/bar@1.0.0-1/lua/bar.lua <<'EOF'
    return { world = function() return "world" end }
    EOF
  '';

  plugin-src = pkgs.stdenv.mkDerivation {
    name = "foo-nvim-src";
    dontUnpack = true;
    installPhase = ''
      mkdir -p $out/lua $out/plugin
      cat > $out/lux.toml <<'EOF'
      package = "foo-nvim"
      version = "1.0.0"
      lua = ">=5.1"

      [description]
      summary = "test plugin"
      license = "MIT"

      [dependencies]
      bar = "*"

      [build]
      type = "builtin"
      copy_directories = ["plugin"]

      [build.modules]
      foo = "lua/foo.lua"
      EOF
      cat > $out/lua/foo.lua <<'EOF'
      local bar = require("bar")
      return { hello = function() return "hello " .. bar.world() end }
      EOF
      cat > $out/plugin/foo.vim <<'EOF'
      lua _G.foo_from_plugin = require("foo").hello()
      EOF
    '';
  };

  foo-nvim =
    (pkgs.buildLuxPackage {
      inherit lua;
      lux-cli = pkgs.lux-cli.debug;
    }) {
      pname = "foo-nvim";
      version = "1.0.0";
      src = plugin-src;
      luxVendorDir = vendored-dep;
    };

  foo-vimplugin = pkgs.toLuxNeovimPlugin foo-nvim;

  nvim = (pkgs.neovim-lux pkgs.lux-luajit.debug).override {plugins = [foo-vimplugin];};
in {
  test = pkgs.runCommandLocal "lux-nvim-plugin-test" {} ''
    output=$(${nvim}/bin/nvim --headless -c 'lua print(_G.foo_from_plugin)' -c 'qa!' 2>&1)
    echo "$output" | grep -q "hello world"
    touch "$out"
  '';
}
