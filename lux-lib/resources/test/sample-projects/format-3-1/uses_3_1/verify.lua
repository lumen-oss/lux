local rockdir, lua_version = arg[1], arg[2]
local expected = assert(io.open("expected-lua-version.txt", "rb"), "missing expected version"):read("*a")
assert(lua_version == expected, "unexpected LUA_VERSION: " .. tostring(lua_version))
assert(io.open(rockdir .. "/src/a_rock.lua", "rb"), "A_ROCK_ROCKDIR is not the installed dependency")
