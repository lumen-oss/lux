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
        let lua_output_path = lua_dir.join(&name);
        let temp_file = lua_dir.join(format!("{}.part", &name));
        writer
            .write_file(&temp_file)
            .unwrap_or_else(|e| panic!("Failed to write {}: {e}", lua_output_path.display()));
        let content = std::fs::read_to_string(&temp_file).unwrap();
        if content.contains("@param param") {
            std::fs::remove_file(&temp_file).unwrap();
            panic!("Generated definitions with undocumented `@param`");
        }
        std::fs::rename(temp_file, &lua_output_path).unwrap();
        println!("Generated {}", &lua_output_path.display());
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
