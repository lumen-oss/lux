use std::path::{Path, PathBuf};

use mlua_extras::typed::{Type, Typed, TypedDataMethods, TypedUserData};
use path_slash::PathBufExt;

use crate::loader;

#[derive(Clone)]
pub(crate) struct DatafileModule;

impl Typed for DatafileModule {
    fn ty() -> Type {
        Type::named("datafile.Module")
    }
}

impl TypedUserData for DatafileModule {
    fn add_methods<M: TypedDataMethods<Self>>(methods: &mut M) {
        methods.document("The data directory of the package that owns the given Lua source file");
        methods.param("source", "Path of a Lua source file in a Lux install tree");
        methods.ret("Directory in which to search for data files");
        methods.ret("Error message if the package could not be determined");
        methods.add_function("dir", |_, source: String| {
            Ok(match data_dir(&source) {
                Some(dir) => (Some(dir.to_slash_lossy().into_owned()), None),
                None => (
                    None,
                    Some("could not find the calling module's Lux data directory".to_string()),
                ),
            })
        });
    }

    fn add_documentation<F: mlua_extras::typed::TypedDataDocumentation<Self>>(docs: &mut F) {
        docs.add("Module for resolving data files installed by Lux packages");
    }
}

impl mlua::UserData for DatafileModule {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(fields);
        <Self as TypedUserData>::add_fields(&mut wrapper);
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        let mut wrapper = mlua_extras::typed::WrappedBuilder::new(methods);
        <Self as TypedUserData>::add_methods(&mut wrapper);
    }
}

#[cfg(feature = "definitions")]
mod definitions_registry {
    use mlua_extras::typed::{Type, TypedClassBuilder};

    use super::DatafileModule;
    use crate::definitions::LuxDefinition;

    inventory::submit! {
        LuxDefinition {
            name: "datafile.Module",
            build: || Type::class(TypedClassBuilder::new::<DatafileModule>().build()),
        }
    }
}

fn data_dir(source: &str) -> Option<PathBuf> {
    let source = Path::new(source.strip_prefix('@').unwrap_or(source));
    let (tree_root, id, package) = loader::owning_package(source)?;
    let etc = loader::rock_root(&tree_root, &id, &package).join("etc");
    etc.is_dir().then_some(etc)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use assert_fs::TempDir;
    use mlua::Lua;

    const ROCK_DIR: &str = "5.1/abcde-foo@1.0.0-1";

    fn lockfile() -> &'static str {
        r#"{
  "version": "2.0.0",
  "rocks": {
    "abcde": {
      "name": "foo",
      "version": "1.0.0-1",
      "source": "local",
      "hashes": {
        "rockspec": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "source": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
      }
    }
  },
  "entrypoints": {}
}"#
    }

    fn setup() -> (TempDir, Lua) {
        let temp = TempDir::new().unwrap();
        let rock = temp.path().join(ROCK_DIR);
        fs::create_dir_all(rock.join("src")).unwrap();
        fs::create_dir_all(rock.join("etc")).unwrap();
        fs::write(temp.path().join("5.1/lux.lock"), lockfile()).unwrap();

        let lua = Lua::new();
        lua.globals().set("lux", crate::lux(&lua).unwrap()).unwrap();
        (temp, lua)
    }

    fn assert_dir(lua: &Lua, source: &str) {
        lua.globals().set("source", source).unwrap();
        lua.load(
            r#"
            local dir = lux.datafile.dir(source)
            assert(dir:match("/etc$"))
            "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn resolves_etc_dir_of_calling_package() {
        let (temp, lua) = setup();
        let module = temp.path().join(ROCK_DIR).join("src/foo.lua");
        fs::write(&module, "").unwrap();

        assert_dir(&lua, &format!("@{}", module.display()));
    }

    #[cfg(unix)]
    #[test]
    fn resolves_package_through_symlinked_layout() {
        let (temp, lua) = setup();
        let link = temp.path().join("link");
        fs::create_dir_all(&link).unwrap();
        std::os::unix::fs::symlink(temp.path().join(ROCK_DIR).join("src"), link.join("lua"))
            .unwrap();
        fs::write(link.join("lua/foo.lua"), "").unwrap();

        assert_dir(&lua, link.join("lua/foo.lua").to_str().unwrap());
    }
}
