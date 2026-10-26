package = "lubmin"
version = "1.0.0-1"
source = {
  url = "git+https://github.com/lubyk/lub",
  tag = "REL-1.1.0",
  dir = "lub",
}
dependencies = { "lua >= 5.1" }
build = {
  type = "builtin",
  modules = { ["lub.Template"] = "lub/Template.lua" },
}
