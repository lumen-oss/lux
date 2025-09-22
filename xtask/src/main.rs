use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use cargo_packager::{
    config::{AppImageConfig, Binary, DebianConfig, PacmanConfig, Resource},
    PackageFormat, SigningConfig,
};
use clap::{CommandFactory, ValueEnum};
use clap_complete::{generate_to, Shell};
use clap_mangen::Man;
use lux_cli::Cli;
use serde::Deserialize;
use strum::IntoEnumIterator;
use xtask_lua::LuaFeature;

type DynError = Box<dyn std::error::Error>;

fn main() {
    if let Err(e) = try_main() {
        eprintln!("{e}");
        std::process::exit(-1);
    }
}

fn try_main() -> Result<(), DynError> {
    let task = env::args().nth(1);

    match task.as_deref() {
        // Assume that the user wants to build the release version
        // when trying to build the distributed version.
        Some("dist") => dist(true)?,
        Some("dist-man") => dist_man()?,
        Some("dist-completions") => dist_completions()?,
        Some("dist-package") => dist_package()?,
        Some("build") => build(BuildOpts {
            release: false,
            vendored: false,
        })?,
        Some("build-release") => build(BuildOpts {
            release: true,
            vendored: false,
        })?,
        _ => print_help(),
    }

    Ok(())
}

fn print_help() {
    eprintln!(
        "Tasks:

build               builds and links all libraries and the application
dist-man            builds man pages
dist-completions    builds shell completions
dist-package        builds binary package distribution(s) for the current platform
dist                builds everything, equivalent to build + dist-man + dist-completions

LUA_LIB_DIR         when set, overrides the path to the directory containing the compiled lux-lua libraries
"
    )
}

fn dist(release: bool) -> Result<(), DynError> {
    build(BuildOpts {
        release,
        vendored: false,
    })?;
    dist_man()?;
    dist_completions()
}

struct BuildOpts {
    release: bool,
    vendored: bool,
}

fn build(opts: BuildOpts) -> Result<(), DynError> {
    let profile = if opts.release { "release" } else { "debug" };

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target_dir = project_root().join("target");

    let mut args = vec![
        "build".into(),
        "--locked".into(),
        "--target-dir".into(),
        target_dir.to_string_lossy().to_string(),
        "--no-default-features".into(),
        "--features".into(),
        "lua54".into(),
    ];

    if opts.vendored {
        args.push("--features".into());
        args.push("vendored".into());
    }

    if opts.release {
        args.push("--release".into());
    }

    let status = Command::new(cargo)
        .current_dir(project_root())
        .args(args)
        .status()?;

    if !status.success() {
        Err("cargo build failed")?;
    }

    let dest_dir = target_dir.join(profile);

    #[cfg(not(target_env = "msvc"))]
    let dest_bin = dest_dir.join("lx");

    #[cfg(target_env = "msvc")]
    let dest_bin = dest_dir.join("lx.exe");

    if !dest_bin.is_file() {
        Err(format!("{} not found", dest_bin.display()))?;
    }
    if opts.release {
        #[cfg(not(target_env = "msvc"))]
        let dist_file = dist_dir().join("lx");

        #[cfg(target_env = "msvc")]
        let dist_file = dist_dir().join("lx.exe");

        fs::create_dir_all(dist_dir())?;
        fs::copy(&dest_bin, dist_file)?;
    }

    if opts.release
        && Command::new("strip")
            .arg("--version")
            .stdout(Stdio::null())
            .status()
            .inspect_err(|_| eprintln!("checking for `strip` utility"))
            .is_ok()
    {
        println!("stripping the binary");
        let status = Command::new("strip").arg(&dest_bin).status()?;
        if !status.success() {
            Err("strip failed")?;
        }
    }

    Ok(())
}

fn dist_man() -> Result<(), DynError> {
    fs::create_dir_all(dist_dir())?;

    let cmd = &mut Cli::command();

    Man::new(cmd.clone())
        .render(&mut File::create(dist_dir().join("lx.1")).unwrap())
        .unwrap();
    Ok(())
}

fn dist_completions() -> Result<(), DynError> {
    fs::create_dir_all(dist_dir())?;

    let cmd = &mut Cli::command();

    for shell in Shell::value_variants() {
        generate_to(*shell, cmd, "lx", dist_dir()).unwrap();
    }

    Ok(())
}

#[derive(Deserialize)]
struct LuxManifest {
    workspace: LuxWorkspace,
}

#[derive(Deserialize)]
struct LuxWorkspace {
    package: LuxPackage,
}

#[derive(Deserialize)]
struct LuxPackage {
    version: String,
}

fn dist_package() -> Result<(), DynError> {
    let signing_config = SigningConfig::new()
        .private_key(std::env::var("LUX_SIGN_PRIVATE_KEY").expect("LUX_SIGN_PRIVATE_KEY not set"))
        .password(
            std::env::var("LUX_SIGN_PRIVATE_KEY_PASS").expect("LUX_SIGN_PRIVATE_KEY_PASS not set"),
        );

    let dist_dir = dist_dir();
    if dist_dir.is_dir() {
        println!("removing {}", dist_dir.display());
        let _ = fs::remove_dir_all(&dist_dir);
    }
    for lua_feature in LuaFeature::iter() {
        let (_, canonical_lua_version) = lua_feature.lua_feature_strs();
        println!("building lux-lua for Lua {canonical_lua_version}...");
        xtask_lua::dist(
            true,
            Some(xtask_lua::DistOpts {
                lua_feature: Some(lua_feature),
                clean_dist_dir: false,
                vendored: true,
            }),
        )?;
    }
    println!("building lux-cli...");
    build(BuildOpts {
        release: true,
        vendored: true,
    })?;
    println!("building man pages...");
    dist_man()?;
    println!("building shell completions...");
    dist_completions()?;
    let project_root = project_root();
    let manifest_path = project_root.join("Cargo.toml");
    if !manifest_path.is_file() {
        Err(format!("{} not found", manifest_path.display()))?;
    }
    let manifest_content = fs::read_to_string(manifest_path)?;
    let manifest: LuxManifest = toml::from_str(&manifest_content)?;

    #[cfg(not(target_env = "msvc"))]
    let lx_bin_path = dist_dir.join("lx");

    #[cfg(target_env = "msvc")]
    let lx_bin_path = dist_dir.join("lx.exe");

    let resources = if cfg!(target_env = "msvc") {
        vec![
            Resource::Single("target/dist/share/lux-lua/".into()),
            Resource::Mapped {
                src: "target/dist/_lx.ps1".into(),
                target: "completions/_lx.ps1".into(),
            },
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            Resource::Single("target/dist/share/lux-lua/".into()),
            Resource::Mapped {
                src: "target/dist/lx.1".into(),
                target: "man/lx.1".into(),
            },
            Resource::Mapped {
                src: "target/dist/_lx.ps1".into(),
                target: "completions/_lx.ps1".into(),
            },
            Resource::Mapped {
                src: "target/dist/lx.bash".into(),
                target: "completions/lx.bash".into(),
            },
            Resource::Mapped {
                src: "target/dist/_lx".into(),
                target: "completions/zsh/_lx".into(),
            },
            Resource::Mapped {
                src: "target/dist/lx.fish".into(),
                target: "completions/lx.fish".into(),
            },
            Resource::Mapped {
                src: "target/dist/lx.elv".into(),
                target: "completions/lx.elv".into(),
            },
        ]
    } else {
        Vec::new()
    };

    if !lx_bin_path.is_file() {
        Err(format!("{} not found", lx_bin_path.display()))?;
    }
    let file_mappings = [
        ("target/dist/share/lux-lua", "usr/share/lux-lua"),
        ("target/dist/lib/pkgconfig", "usr/lib/pkgconfig"),
        ("target/dist/lx.1", "usr/share/man/man1/lx.1"),
        ("target/dist/_lx", "usr/share/zsh/site_functions/_lx"),
        (
            "target/dist/lx.bash",
            "usr/share/bash-completion/completions/lx.bash",
        ),
        (
            "target/dist/lx.fish",
            "usr/share/fish/vendor_completions.d/lx.fish",
        ),
        (
            "target/dist/lx.elv",
            "usr/share/elvish/lib/completions/lx.elv",
        ),
    ];

    let icons = if cfg!(target_os = "macos") {
        Vec::new()
    } else {
        vec!["lux-logo.svg", "lux-logo_32.png"]
    };

    let config_builder = cargo_packager::Config::builder()
        .product_name("lux-cli")
        .version(manifest.workspace.package.version)
        .out_dir(&dist_dir)
        .binaries(vec![Binary::new(lx_bin_path).main(true)])
        .resources(resources)
        .description("A luxurious package manager for Lua")
        .homepage("https://lux.lumen-labs.org")
        .authors(vec!["mrcjkb", "vhyrro"])
        .publisher("lumen-oss")
        .identifier("org.neorocks.lux")
        .license_file(project_root.join("LICENSE"))
        .icons(icons)
        .appimage(AppImageConfig::new().files(file_mappings))
        .pacman(
            PacmanConfig::new()
                .conflicts(["lux-cli-git", "lux-cli"])
                .depends(["glibc", "gcc-libs", "bzip2", "xz"])
                .provides(["lx"])
                .files(file_mappings),
        )
        .deb(DebianConfig::new().files(file_mappings))
        .formats(vec![PackageFormat::All])
        .log_level(cargo_packager::config::LogLevel::Trace);
    // NOTE: The AppImage/linuxdeploy-<target>.AppImage will fail on NixOS.
    println!("building binary package...");
    cargo_packager::package_and_sign(config_builder.config(), &signing_config)
        .inspect_err(|err| eprintln!("failed to package lux:\n{err:?}"))?;
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
