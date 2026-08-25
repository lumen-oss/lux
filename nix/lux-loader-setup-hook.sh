export LUA_INIT="require('lux').loader()${LUA_INIT:+;$LUA_INIT}"

luxEnvHook() {
    local dir="$1"
    local treeDir="$dir/share/lux/@luaversion@"
    if [ ! -d "$treeDir" ]; then return; fi
    case ";${LUA_PATH-};" in
        *";$treeDir/?.lua;"*) ;;
        *) export LUA_PATH="${LUA_PATH:+$LUA_PATH;}$treeDir/?.lua" ;;
    esac
}
addEnvHooks "$hostOffset" luxEnvHook
