use std::{
    env::{self, consts::DLL_EXTENSION},
    fs::{self},
    path::{Path, PathBuf},
    process::Command,
};
use strum_macros::EnumIter;
use target_lexicon::Triple;

pub type DynError = Box<dyn std::error::Error>;

#[derive(EnumIter, PartialEq, Eq)]
pub enum LuaFeature {
    Lua51,
    Lua52,
    Lua53,
    Lua54,
    Lua55,
    Luajit,
}

impl LuaFeature {
    pub fn lua_feature_strs(&self) -> (&str, &str) {
        match self {
            LuaFeature::Lua51 => ("lua51", "5.1"),
            LuaFeature::Lua52 => ("lua52", "5.2"),
            LuaFeature::Lua53 => ("lua53", "5.3"),
            LuaFeature::Lua54 => ("lua54", "5.4"),
            LuaFeature::Lua55 => ("lua55", "5.5"),
            LuaFeature::Luajit => ("luajit", "jit"),
        }
    }
}

pub struct DistOpts {
    pub lua_feature: Option<LuaFeature>,
    /// Whether to delete the `target/dist` directory
    pub clean_dist_dir: bool,
    /// Whether to enable the vendored feature
    pub vendored: bool,
}

impl Default for DistOpts {
    fn default() -> Self {
        Self {
            lua_feature: None,
            clean_dist_dir: true,
            vendored: false,
        }
    }
}

pub fn dist(release: bool, opts: Option<DistOpts>) -> Result<(), DynError> {
    let opts = opts.unwrap_or_default();
    let dist_dir = dist_dir();
    if opts.clean_dist_dir && dist_dir.is_dir() {
        println!("removing {}", dist_dir.display());
        let _ = fs::remove_dir_all(&dist_dir);
    }
    println!("creating {}", dist_dir.display());
    fs::create_dir_all(&dist_dir)?;

    let profile = if release { "release" } else { "debug" };

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let project_root = project_root();

    let lua_feature = match opts.lua_feature {
        Some(lua_feature) => lua_feature,
        None => {
            if cfg!(feature = "lua51") {
                LuaFeature::Lua51
            } else if cfg!(feature = "lua52") {
                LuaFeature::Lua52
            } else if cfg!(feature = "lua53") {
                LuaFeature::Lua53
            } else if cfg!(feature = "lua54") {
                LuaFeature::Lua54
            } else if cfg!(feature = "lua55") {
                LuaFeature::Lua55
            } else if cfg!(feature = "luajit") {
                LuaFeature::Luajit
            } else {
                Err("No Lua version feature enabled")?
            }
        }
    };

    let target_dir = project_root.join("target");

    let (lua_feature_flag, canonical_lua_version) = lua_feature.lua_feature_strs();

    let mut args = vec![
        "build".into(),
        "--package".into(),
        "lux-lua".into(),
        "--locked".into(),
        "--target-dir".into(),
        target_dir.to_string_lossy().to_string(),
        "--no-default-features".into(),
        "--features".into(),
        lua_feature_flag.into(),
    ];

    if opts.vendored {
        args.push("--features".into());
        args.push("vendored".into());
    }

    if release {
        args.push("--release".into());
    }

    let status = Command::new(&cargo)
        .current_dir(&project_root)
        .args(args)
        .status()?;

    if !status.success() {
        Err("cargo build failed")?;
    }

    let dest_dir = target_dir.join(profile);
    let output_dir = if release { dist_dir } else { dest_dir.clone() };

    let lib_dir = output_dir
        .join("share")
        .join("lux-lua")
        .join(canonical_lua_version);
    if opts.clean_dist_dir && lib_dir.is_dir() {
        println!("removing {}", lib_dir.display());
        let _ = fs::remove_dir_all(&lib_dir);
    }
    println!("creating {}", lib_dir.display());
    fs::create_dir_all(&lib_dir)?;

    let target_profile_dir = project_root.join(format!("target/{profile}"));

    println!("{} contents:", target_profile_dir.display());
    for entry in fs::read_dir(&target_profile_dir)?.filter_map(Result::ok) {
        println!("{}", entry.file_name().display());
    }

    let host = Triple::host();
    #[cfg(not(target_env = "msvc"))]
    let (src_file, dest_file, dest_arch_file) = (
        target_profile_dir.join(format!("liblux_lua.{DLL_EXTENSION}")),
        lib_dir.join("lux.so"),
        output_dir.join(format!("lux-{lua_feature_flag}-{host}.so")),
    );

    #[cfg(target_env = "msvc")]
    let (src_file, dest_file, dest_arch_file) = (
        target_profile_dir.join(format!("lux_lua.{DLL_EXTENSION}")),
        lib_dir.join(format!("lux.{DLL_EXTENSION}")),
        output_dir.join(format!("lux-{lua_feature_flag}-{host}.{DLL_EXTENSION}")),
    );

    if !src_file.is_file() {
        Err(format!("{} not found", src_file.display()))?;
    }

    println!("copying {} to {}", src_file.display(), dest_file.display());
    fs::copy(&src_file, &dest_file)?;
    println!(
        "copying {} to {}",
        src_file.display(),
        dest_arch_file.display()
    );
    fs::copy(&src_file, &dest_arch_file)?;

    let version = {
        let manifest_path = project_root.join("Cargo.toml");
        let manifest = fs::read_to_string(manifest_path)?;
        let package: toml::Value = toml::from_str(&manifest)?;
        package["workspace"]["package"]["version"]
            .as_str()
            .ok_or("lux-lua: Failed to get version from Cargo.toml")?
            .to_string()
    };

    // Create and write the pkg-config file
    let pkg_config_dir = output_dir.join("lib").join("pkgconfig");
    println!("creating {}", pkg_config_dir.display());
    fs::create_dir_all(&pkg_config_dir)?;

    let lua_full_name = if canonical_lua_version == "jit" {
        "luajit".to_string()
    } else {
        format!("Lua {canonical_lua_version}")
    };

    let pc_content = format!(
        r#"prefix=${{pcfiledir}}/../../share/lux-lua/{canonical_lua_version}
exec_prefix=${{prefix}}
libdir=${{prefix}}
luaversion={canonical_lua_version}

Name: lux-lua{canonical_lua_version}
Description: Lux API for {lua_full_name}
Version: {version}
Cflags:
Libs: -L${{libdir}}"#
    );

    let pc_file = pkg_config_dir.join(format!("lux-lua{canonical_lua_version}.pc"));
    println!("writing {}", pc_file.display());
    fs::write(pc_file, pc_content)?;

    Ok(())
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}

fn dist_dir() -> PathBuf {
    project_root().join("target/dist")
}
