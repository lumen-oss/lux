use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

use lux_lib::lockfile::{LocalPackageId, Lockfile, ReadOnly};
use mlua::prelude::*;
use path_absolutize::Absolutize;

// NOTE: The loader runs on the Lua thread.
thread_local! {
    static CACHE: RefCell<LoaderCache> = RefCell::new(LoaderCache::default());
}

#[derive(Default)]
struct LoaderCache {
    trees: Option<(String, Rc<Vec<PathBuf>>)>,
    lockfiles: HashMap<PathBuf, Rc<Lockfile<ReadOnly>>>,
}

fn current_file(lua: &Lua) -> Option<String> {
    lua.inspect_stack(2, |debug| {
        debug
            .source()
            .source
            .as_deref()
            .and_then(|source| source.strip_prefix('@'))
            .map(str::to_string)
    })?
}

fn load_file(lua: &Lua, module: &str, path: &Path) -> mlua::Result<Option<mlua::Function>> {
    let module_path = module.replace('.', std::path::MAIN_SEPARATOR_STR);

    #[cfg(not(target_env = "msvc"))]
    let c_dylib_extension = "so";

    #[cfg(target_env = "msvc")]
    let c_dylib_extension = "dll";

    let src_lua = path.join("src").join(format!("{module_path}.lua"));
    let src_init = path.join("src").join(&module_path).join("init.lua");
    let lib = path
        .join("lib")
        .join(format!("{module_path}.{c_dylib_extension}"));

    if let Some(file) = [src_lua, src_init].into_iter().find(|file| file.exists()) {
        lua.create_function(move |lua, ()| {
            let dofile: mlua::Function = lua.globals().get("dofile")?;
            dofile.call::<mlua::Value>(file.clone())
        })
        .map(Some)
    } else if lib.is_file() {
        let c_open = format!("luaopen_{}", module.replace('.', "_"));
        let package: mlua::Table = lua.globals().get("package")?;
        let loadlib: mlua::Function = package.get("loadlib")?;
        let r = loadlib.call::<mlua::Value>((lib, c_open))?;
        Ok(match r {
            mlua::Value::Function(loader) => Some(loader),
            _ => None,
        })
    } else {
        Ok(None)
    }
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

fn cached_lockfile(tree_root: &Path) -> Option<Rc<Lockfile<ReadOnly>>> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(lockfile) = cache.lockfiles.get(tree_root) {
            return Some(Rc::clone(lockfile));
        }
        // A `lux.lock` could be a workspace lockfile rather than a tree lockfile.
        // We ignore it if it fails to parse.
        let lockfile = Rc::new(Lockfile::load(tree_root.join("lux.lock"), None).ok()?);
        cache
            .lockfiles
            .insert(tree_root.to_path_buf(), Rc::clone(&lockfile));
        Some(lockfile)
    })
}

fn find_trees_from_package_path(lua: &Lua) -> mlua::Result<Rc<Vec<PathBuf>>> {
    let package: LuaTable = lua.globals().get("package")?;
    let path: String = package.get("path")?;
    let cpath: String = package.get("cpath")?;
    let signature = format!("{path}\u{0}{cpath}");

    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((cached_signature, trees)) = &cache.trees {
            if cached_signature == &signature {
                return Ok(Rc::clone(trees));
            }
        }

        let mut trees = Vec::new();
        let mut seen = HashSet::new();
        for dir in path
            .split(';')
            .chain(cpath.split(';'))
            .map(PathBuf::from)
            .filter_map(|p| p.parent().map(|p| p.to_path_buf()))
        {
            for ancestor in dir.ancestors() {
                if ancestor.join("lux.lock").exists() {
                    let tree_root = ancestor.to_path_buf();
                    if seen.insert(tree_root.clone()) {
                        trees.push(tree_root);
                    }
                    break;
                }
            }
        }

        let trees = Rc::new(trees);
        cache.trees = Some((signature, Rc::clone(&trees)));
        Ok(trees)
    })
}

fn load_from_workspace_tree(
    lua: &Lua,
    module: &str,
    current_file: &Path,
) -> mlua::Result<Option<mlua::Function>> {
    let Some(tree_root) = current_file
        .ancestors()
        .find(|path| path.join("lux.lock").exists())
    else {
        return Ok(None);
    };

    let Some(lockfile) = cached_lockfile(tree_root) else {
        return Ok(None);
    };

    // The package directory in a Lux tree is named `<id>-<name>-<version>`, so the hash is the
    // first `-`-separated component relative to the tree root.
    let Some(module_hash) = current_file
        .strip_prefix(tree_root)
        .ok()
        .and_then(|rel| rel.components().next())
        .and_then(|component| component.as_os_str().to_str())
        .and_then(|component| component.split('-').next())
    else {
        return Ok(None);
    };

    // NOTE(vhyrro): On safety - it's possible that the user *could* tamper
    // with the lux tree and malform the package hash. In this case, this
    // should never cause any security-related problems anyway, as we'll
    // crash right after this function returns None.
    let Some(package) =
        lockfile.get(unsafe { &LocalPackageId::from_unchecked(module_hash.to_string()) })
    else {
        return Ok(None);
    };

    let Some(dep_id) = package.dependencies().iter().copied().find_map(|id| {
        let dep = lockfile.get(id)?;
        (dep.name().to_string() == module).then_some(id)
    }) else {
        return Ok(None);
    };

    let Some(dep) = lockfile.get(dep_id) else {
        return Ok(None);
    };

    let path = tree_root.join(format!("{dep_id}-{}@{}", dep.name(), dep.version()));
    load_file(lua, module, &path)
}

fn load_from_installed_tree(lua: &Lua, module: &str) -> mlua::Result<Option<mlua::Function>> {
    for tree_root in find_trees_from_package_path(lua)?.iter() {
        if let Some(lockfile) = cached_lockfile(tree_root) {
            for (id, package) in lockfile.rocks() {
                let path = tree_root.join(format!("{id}-{}@{}", package.name(), package.version()));
                if let Some(loader) = load_file(lua, module, &path)? {
                    return Ok(Some(loader));
                }
            }
        };
    }
    Ok(None)
}

pub fn loader(lua: &Lua, module: String) -> mlua::Result<Option<mlua::Function>> {
    if let Some(current_file) = current_file(lua) {
        if let Ok(current_file) = PathBuf::from(current_file).absolutize() {
            if let Some(loader) = load_from_workspace_tree(lua, &module, &current_file)? {
                return Ok(Some(loader));
            }
        }
    }

    load_from_installed_tree(lua, &module)
}

#[cfg(test)]
mod tests {
    use assert_fs::{
        prelude::{FileWriteStr, PathChild},
        TempDir,
    };
    use mlua::Lua;

    use super::load_loader;

    const FOO_HASH: &str = "aaaaa";
    const MAIN_HASH: &str = "bbbbb";

    fn lockfile() -> String {
        format!(
            r#"{{
  "version": "1.0.0",
  "rocks": {{
    "{FOO_HASH}": {{
      "name": "foo",
      "version": "1.0.0-1",
      "pinned": false,
      "opt": false,
      "dependencies": [],
      "constraint": null,
      "binaries": [],
      "source": "local",
      "hashes": {{
        "rockspec": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "source": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
      }}
    }}
  }},
  "entrypoints": []
}}"#
        )
    }

    fn workspace_lockfile() -> String {
        format!(
            r#"{{
  "version": "1.0.0",
  "rocks": {{
    "{MAIN_HASH}": {{
      "name": "main",
      "version": "1.0.0-1",
      "pinned": false,
      "opt": false,
      "dependencies": ["{FOO_HASH}"],
      "constraint": null,
      "binaries": [],
      "source": "local",
      "hashes": {{
        "rockspec": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "source": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
      }}
    }},
    "{FOO_HASH}": {{
      "name": "foo",
      "version": "1.0.0-1",
      "pinned": false,
      "opt": false,
      "dependencies": [],
      "constraint": null,
      "binaries": [],
      "source": "local",
      "hashes": {{
        "rockspec": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "source": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
      }}
    }}
  }},
  "entrypoints": []
}}"#
        )
    }

    fn setup_tree() -> (TempDir, Lua) {
        let tree = TempDir::new().unwrap();
        tree.child("5.1")
            .child("lux.lock")
            .write_str(&lockfile())
            .unwrap();
        tree.child("5.1")
            .child(format!("{FOO_HASH}-foo@1.0.0-1"))
            .child("src")
            .child("foo.lua")
            .write_str("_G.foo_loaded = 'yes'\n")
            .unwrap();

        let lua = Lua::new();
        load_loader(&lua).unwrap();
        (tree, lua)
    }

    fn set_package_path(tree: &TempDir, lua: &Lua) {
        let src_dir = tree
            .path()
            .join("5.1")
            .join(format!("{FOO_HASH}-foo@1.0.0-1"))
            .join("src");
        lua.globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .set("path", format!("{}/?.lua", src_dir.display()))
            .unwrap();
    }

    #[test]
    fn test_load_from_package_path() {
        let (tree, lua) = setup_tree();
        set_package_path(&tree, &lua);

        lua.load("require('foo')").exec().unwrap();
        let foo_loaded: String = lua.globals().get("foo_loaded").unwrap();
        assert_eq!(foo_loaded, "yes");
    }

    #[test]
    fn test_load_from_cpath() {
        let (tree, lua) = setup_tree();

        let lib_dir = tree
            .path()
            .join("5.1")
            .join(format!("{FOO_HASH}-foo@1.0.0-1"))
            .join("lib");
        let package = lua.globals().get::<mlua::Table>("package").unwrap();
        package.set("path", "").unwrap();
        package
            .set("cpath", format!("{}/?.so", lib_dir.display()))
            .unwrap();

        lua.load("require('foo')").exec().unwrap();
        let foo_loaded: String = lua.globals().get("foo_loaded").unwrap();
        assert_eq!(foo_loaded, "yes");
    }

    #[test]
    fn test_load_module_returning_function() {
        let tree = TempDir::new().unwrap();
        tree.child("5.1")
            .child("lux.lock")
            .write_str(&lockfile())
            .unwrap();
        tree.child("5.1")
            .child(format!("{FOO_HASH}-foo@1.0.0-1"))
            .child("src")
            .child("foo.lua")
            .write_str("return function() return 42 end\n")
            .unwrap();

        let lua = Lua::new();
        load_loader(&lua).unwrap();
        set_package_path(&tree, &lua);

        let result: i64 = lua.load("return require('foo')()").eval().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_load_nested_module() {
        let tree = TempDir::new().unwrap();
        tree.child("5.1")
            .child("lux.lock")
            .write_str(&lockfile())
            .unwrap();
        tree.child("5.1")
            .child(format!("{FOO_HASH}-foo@1.0.0-1"))
            .child("src")
            .child("foo")
            .child("bar.lua")
            .write_str("return 'nested'\n")
            .unwrap();

        let lua = Lua::new();
        load_loader(&lua).unwrap();
        set_package_path(&tree, &lua);

        let result: String = lua.load("return require('foo.bar')").eval().unwrap();
        assert_eq!(result, "nested");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn test_load_c_module_from_built_project() {
        use lux_lib::{
            config::ConfigBuilder, lua_version::LuaVersion, operations::InstallProject,
            path::Paths, project::Project, tree::InstallTree,
        };

        let sample = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../lux-lib/resources/test/sample-projects/c-src/");
        let project_dir = TempDir::new().unwrap();
        assert_fs::fixture::PathCopy::copy_from(&project_dir, &sample, &["**"]).unwrap();

        let project = Project::from_exact(project_dir.path()).unwrap().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap();
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();
        InstallProject::new()
            .project(&project)
            .config(&config)
            .tree(&tree)
            .build()
            .await
            .unwrap();

        let lua = unsafe {
            // Required to load C modules
            Lua::unsafe_new()
        };
        load_loader(&lua).unwrap();
        let paths = Paths::new(&tree).unwrap();
        let package = lua.globals().get::<mlua::Table>("package").unwrap();
        package.set("path", paths.package_path().joined()).unwrap();
        package
            .set("cpath", paths.package_cpath().joined())
            .unwrap();

        lua.load("require('foo').printok()").exec().unwrap();
    }

    #[test]
    fn test_load_from_workspace_tree() {
        let tree = TempDir::new().unwrap();
        tree.child("tree")
            .child("5.1")
            .child("lux.lock")
            .write_str(&workspace_lockfile())
            .unwrap();
        tree.child("tree")
            .child("5.1")
            .child(format!("{FOO_HASH}-foo@1.0.0-1"))
            .child("src")
            .child("foo.lua")
            .write_str("_G.foo_loaded = 'yes'\n")
            .unwrap();

        let entrypoint = tree
            .child("tree")
            .child("5.1")
            .child(format!("{MAIN_HASH}-main@1.0.0-1"))
            .child("src")
            .child("main.lua");
        entrypoint.write_str("require('foo')\n").unwrap();

        let lua = Lua::new();
        load_loader(&lua).unwrap();
        lua.load(entrypoint.path()).exec().unwrap();
        let foo_loaded: String = lua.globals().get("foo_loaded").unwrap();
        assert_eq!(foo_loaded, "yes");
    }

    #[test]
    fn test_source_file_skips_workspace_lockfile() {
        let tree = TempDir::new().unwrap();
        tree.child("lux.lock")
            .write_str(r#"{"version":"1.0.0","dependencies":{"rocks":{},"entrypoints":[]}}"#)
            .unwrap();
        tree.child("tree")
            .child("5.1")
            .child("lux.lock")
            .write_str(&lockfile())
            .unwrap();
        tree.child("tree")
            .child("5.1")
            .child(format!("{FOO_HASH}-foo@1.0.0-1"))
            .child("src")
            .child("foo.lua")
            .write_str("_G.foo_loaded = 'yes'\n")
            .unwrap();
        tree.child("test")
            .child("spec.lua")
            .write_str("require('foo')\n")
            .unwrap();

        let lua = Lua::new();
        load_loader(&lua).unwrap();
        let src_dir = tree
            .path()
            .join("tree")
            .join("5.1")
            .join(format!("{FOO_HASH}-foo@1.0.0-1"))
            .join("src");
        lua.globals()
            .get::<mlua::Table>("package")
            .unwrap()
            .set("path", format!("{}/?.lua", src_dir.display()))
            .unwrap();

        lua.load(tree.path().join("test").join("spec.lua"))
            .exec()
            .unwrap();
        let foo_loaded: String = lua.globals().get("foo_loaded").unwrap();
        assert_eq!(foo_loaded, "yes");
    }
}
