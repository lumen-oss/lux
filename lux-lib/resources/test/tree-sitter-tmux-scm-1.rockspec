local git_ref = '72be3b7819bd6ca095f1f4ce365ba89d801d577c'
local modrev = 'scm'
local specrev = '1'

local repo_url = 'https://github.com/Freed-Wu/tree-sitter-tmux'

rockspec_format = '3.0'
package = 'tree-sitter-tmux'
version = modrev ..'-'.. specrev

description = {
  summary = 'tree-sitter parser for tmux',
  labels = { 'neovim', 'tree-sitter' } ,
  homepage = 'https://github.com/Freed-Wu/tree-sitter-tmux',
  license = 'UNKNOWN'
}

dependencies = { 'lua >= 5.1' }

build_dependencies = {
  'luarocks-build-treesitter-parser >= 5.0.0',
}

source = {
  url = repo_url .. '/archive/' .. git_ref .. '.zip',
  dir = 'tree-sitter-tmux-' .. git_ref,
}

build = {
  type = "treesitter-parser",
  lang = "tmux",
  parser = true,
  generate = false,
}
