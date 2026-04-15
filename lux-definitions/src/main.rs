use std::{env, fs, path::PathBuf};

use mlua_extras::typed::generator::{DefinitionFileGenerator, LuauDefinitionFileGenerator};

fn main() {
    let output_dir = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("definitions"));

    fs::create_dir_all(&output_dir)
        .unwrap_or_else(|e| panic!("Failed to create output directory: {e}"));

    let definitions = lux_lua::definitions::definitions();

    // LuaLS-compatible .d.lua files
    let lua_dir = output_dir.join("lua");
    fs::create_dir_all(&lua_dir)
        .unwrap_or_else(|e| panic!("Failed to create lua output directory: {e}"));
    let gen = DefinitionFileGenerator::new(definitions.clone());
    for (name, writer) in gen.iter() {
        let path = lua_dir.join(&name);
        writer
            .write_file(&path)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
        println!("Generated {}", path.display());
    }

    // Luau .d.luau files
    let luau_dir = output_dir.join("luau");
    fs::create_dir_all(&luau_dir)
        .unwrap_or_else(|e| panic!("Failed to create luau output directory: {e}"));
    let luau_gen = LuauDefinitionFileGenerator::new(definitions);
    for (name, writer) in luau_gen.iter() {
        let path = luau_dir.join(&name);
        writer
            .write_file(&path)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
        println!("Generated {}", path.display());
    }
}
