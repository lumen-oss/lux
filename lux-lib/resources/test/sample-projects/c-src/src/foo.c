#include <lua.h>
#include <lauxlib.h>
#include <lualib.h>

static int l_print_ok(lua_State *L) {
    printf("OK\n");
    return 0;
}

static const struct luaL_Reg mymodule[] = {
    {"printok", l_print_ok},
    {NULL, NULL}
};

#if LUA_VERSION_NUM == 501
static void luaL_setfuncs(lua_State *L, const luaL_Reg *l, int nup) {
    luaL_register(L, NULL, l);
}
#endif

int luaopen_foo(lua_State *L) {
    lua_newtable(L);
    luaL_setfuncs(L, mymodule, 0);
    return 1;
}
