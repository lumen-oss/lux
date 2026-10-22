use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    str::FromStr,
};

use thiserror::Error;

use crate::lua_rockspec::{LuaModule, ModuleSpec, ParseLuaModuleError};

/// Compute the module map for a wally package's contents.
pub(crate) fn modules_from_zip(
    bytes: &[u8],
) -> Result<HashMap<LuaModule, ModuleSpec>, WallyModulesError> {
    let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes))?;
    let mut modules = HashMap::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let path = Path::new(&name);
        if !is_lua_path(path) || is_test_file(path) {
            continue;
        }
        modules.insert(
            lua_module_from_entry(path)?,
            ModuleSpec::SourcePath(path.to_path_buf()),
        );
    }
    Ok(modules)
}

fn is_lua_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("lua" | "luau")
    )
}

fn is_test_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.ends_with(".test") || stem.ends_with(".spec"))
}

fn lua_module_from_entry(path: &Path) -> Result<LuaModule, ParseLuaModuleError> {
    let stripped: PathBuf = match path.components().next() {
        Some(component)
            if matches!(component.as_os_str().to_str(), Some("src" | "lua" | "lib")) =>
        {
            path.components().skip(1).collect()
        }
        _ => path.to_path_buf(),
    };
    if stripped
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        let mut file = stripped;
        file.set_extension("");
        LuaModule::from_pathbuf(file)
    } else {
        let mut module = LuaModule::from_pathbuf(stripped.to_path_buf())?;
        if matches!(
            stripped.file_name().and_then(|name| name.to_str()),
            Some("init.lua" | "init.luau")
        ) {
            module = module.join(unsafe { &LuaModule::from_str("init").unwrap_unchecked() });
        }
        Ok(module)
    }
}

#[derive(Debug, Error)]
pub enum WallyModulesError {
    #[error("failed to read package contents archive")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid module path in package contents")]
    Module(#[from] ParseLuaModuleError),
}
