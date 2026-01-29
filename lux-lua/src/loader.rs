use std::path::PathBuf;

use lux_lib::lockfile::{LocalPackageId, Lockfile};
use mlua::prelude::*;
use path_absolutize::Absolutize;

const fn lua_version() -> &'static str {
    if cfg!(feature = "lua51") {
        "5.1"
    } else if cfg!(feature = "lua52") {
        "5.2"
    } else if cfg!(feature = "lua53") {
        "5.3"
    } else if cfg!(feature = "lua54") {
        "5.4"
    } else if cfg!(feature = "lua55") {
        "5.5"
    } else {
        unreachable!()
    }
}

fn current_file(lua: &Lua) -> Option<String> {
    lua.inspect_stack(2, |debug| {
        debug
            .source()
            .short_src
            .map(|short_src| short_src.to_string())
    })?
}

fn load_file(lua: &Lua, module: String, path: PathBuf) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, ()| {
        // We need to handle 3 possible cases:
        // - `src/?.lua`
        // - `src/?/init.lua`
        // - `src/?.so`

        let module_path = module.replace('.', std::path::MAIN_SEPARATOR_STR);

        #[cfg(not(target_env = "msvc"))]
        let c_dylib_extension = "so";

        #[cfg(target_env = "msvc")]
        let c_dylib_extension = "dll";

        if path
            .join("src")
            .join(format!("{}.lua", module_path))
            .exists()
        {
            lua.load("dofile")
                .call::<()>(path.join("src").join(format!("{}.lua", module_path)))
        } else if path
            .join("src")
            .join(&module_path)
            .join("init.lua")
            .exists()
        {
            lua.load("dofile")
                .call::<()>(path.join("src").join(&module_path).join("init.lua"))
        } else if path
            .join("lib")
            .join(format!("{}.{}", module_path, c_dylib_extension))
            .exists()
        {
            lua.load("dofile").call::<()>(
                path.join("lib")
                    .join(format!("{}.{}", module_path, c_dylib_extension)),
            )
        } else {
            Err(mlua::Error::RuntimeError(format!(
                "module not found: {}",
                module
            )))
        }
    })
}

pub fn load_loader(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let package: LuaTable = globals.get("package")?;
    #[cfg(feature = "lua51")]
    let loaders: LuaTable = package.get("loaders")?;
    #[cfg(not(feature = "lua51"))]
    let loaders: LuaTable = package.get("searchers")?;
    loaders.raw_insert(1, lua.create_function(loader)?)?;

    Ok(())
}

pub fn loader(lua: &Lua, module: String) -> mlua::Result<Option<mlua::Function>> {
    let current_file = match current_file(lua) {
        None => return Ok(None),
        Some(current_file) if current_file == "stdin" => return Ok(None),
        Some(current_file) => PathBuf::from(current_file),
    };
    let current_file = current_file.absolutize().into_lua_err()?;

    // Check if we're in a tree by searching the path for a `.lux` directory
    if let Some(lock_path) = current_file
        .ancestors()
        .find(|path| path.file_name().is_some_and(|filename| filename == ".lux"))
    {
        // Get the name of the current module, so we can look up its dependencies in the lockfile.
        let lockfile = Lockfile::load(lock_path.join("lux.lock"), None).into_lua_err()?;

        // If we're in a lux tree, the path looks like `.lux/5.4/12345678-package_name-1.0.0/src/code.lua`
        // we need to extract the hash from the path.

        let module_hash = current_file.iter().rev().find_map(|path| {
            if let [hash, _name, _version, _rest @ ""] =
                path.to_str().unwrap().splitn(3, '-').collect::<Vec<&str>>()[..4]
            {
                Some(hash)
            } else {
                None
            }
        });

        if let Some(module_hash) = module_hash {
            // NOTE(vhyrro): On safety - it's possible that the user *could* tamper
            // with the lux tree and malform the package hash. In this case, this
            // should never cause any security-related problems anyway, as we'll
            // crash right after this function returns None.
            if let Some(package) =
                lockfile.get(unsafe { &LocalPackageId::from_unchecked(module_hash.to_string()) })
            {
                return package
                    .dependencies()
                    .iter()
                    .find_map(|dep| {
                        let dep = lockfile.get(dep).unwrap();

                        if dep.name().to_string() == module {
                            Some(dep)
                        } else {
                            None
                        }
                    })
                    .map(|dep| {
                        let path = lock_path.parent().unwrap().join(format!(
                            ".lux/{version}/{id}-{package_name}-{package_version}/",
                            version = lua_version(),
                            id = dep.id(),
                            package_name = dep.name(),
                            package_version = dep.version()
                        ));

                        load_file(lua, module, path)
                    })
                    .transpose();
            }
        }

        Ok(None)
    } else {
        Ok(None)
    }
}
