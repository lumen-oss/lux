use lux_workspace_hack as _;

use std::{
    env,
    fs::{self},
    io::{self},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use cargo_packager::{
    config::{AppImageConfig, Binary, DebianConfig, PacmanConfig, Resource},
    PackageFormat, SigningConfig,
};
use flate2::{write::GzEncoder, Compression};
use serde::Deserialize;
use strum::IntoEnumIterator;
use target_lexicon::Triple;
use xtask_lua::LuaFeature;

mod static_gpg;

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
        Some("dist-man") => dist_man(&BuildOpts {
            release: false,
            vendored: true,
        })?,
        Some("dist-completions") => dist_completions(&BuildOpts {
            release: false,
            vendored: true,
        })?,
        Some("dist-package") => dist_package(BuildOpts {
            release: true,
            vendored: true,
        })?,
        Some("dist-package-dry-run") => dist_package(BuildOpts {
            release: false,
            vendored: true,
        })?,
        Some("build") => build(&BuildOpts {
            release: false,
            vendored: false,
        })?,
        Some("build-release") => build(&BuildOpts {
            release: true,
            vendored: false,
        })?,
        Some("gen-definitions") => gen_definitions()?,
        Some("build-static-gpg") => {
            static_gpg::ensure_installed()?;
        }
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
build-static-gpg    builds static GnuPG dependencies
dist-package        builds binary package distribution(s) for the current platform
dist                builds everything, equivalent to build + dist-man + dist-completions
gen-definitions     generates Lua/Luau type definition files into definitions/

LUA_LIB_DIR         when set, overrides the path to the directory containing the compiled lux-lua libraries
"
    )
}

fn dist(release: bool) -> Result<(), DynError> {
    let opts = BuildOpts {
        release,
        vendored: false,
    };
    build(&opts)?;
    dist_man(&opts)?;
    dist_completions(&opts)
}

#[derive(Clone)]
struct BuildOpts {
    release: bool,
    vendored: bool,
}

fn features(opts: &BuildOpts) -> Vec<String> {
    let mut features = vec!["luau-transpile".to_string(), "wally".to_string()];

    if gpgme_enabled() {
        features.push("gpgme".to_string());
    }
    if opts.vendored {
        features.push("vendored".to_string());
    }
    features
}

fn gpgme_enabled() -> bool {
    !cfg!(target_env = "msvc") && !build_target().is_some_and(|target| target.contains("-freebsd"))
}

fn push_features(args: &mut Vec<String>, opts: &BuildOpts) {
    let features = features(opts);
    if !features.is_empty() {
        args.push("--features".into());
        args.push(features.join(","));
    }
}

fn build(opts: &BuildOpts) -> Result<(), DynError> {
    let profile = if opts.release { "release" } else { "debug" };

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let target_dir = project_root().join("target");

    let mut args = vec![
        "build".into(),
        "--locked".into(),
        "--target-dir".into(),
        target_dir.to_string_lossy().to_string(),
        "--no-default-features".into(),
    ];

    push_features(&mut args, opts);

    if opts.release {
        args.push("--release".into());
    }

    let mut command = Command::new(cargo);
    command.current_dir(project_root()).args(args);
    apply_cross_env(&mut command);
    static_gpg::apply(&mut command)?;
    let status = command.status()?;

    if !status.success() {
        Err("cargo build failed")?;
    }

    let dest_dir = match build_target() {
        Some(target) => target_dir.join(target).join(profile),
        None => target_dir.join(profile),
    };

    #[cfg(not(target_env = "msvc"))]
    let dest_bin = dest_dir.join("lx");

    #[cfg(target_env = "msvc")]
    let dest_bin = dest_dir.join("lx.exe");

    if !dest_bin.is_file() {
        Err(format!("{} not found", dest_bin.display()))?;
    }

    #[cfg(not(target_env = "msvc"))]
    let dist_file = dist_dir().join("lx");

    #[cfg(target_env = "msvc")]
    let dist_file = dist_dir().join("lx.exe");

    fs::create_dir_all(dist_dir()).map_err(|err| {
        io::Error::other(format!(
            "error creating directory '{}': {}",
            dist_dir().display(),
            err
        ))
    })?;
    fs::copy(&dest_bin, &dist_file).map_err(|err| {
        io::Error::other(format!(
            "error copying '{}' to '{}': {}",
            dest_bin.display(),
            dist_file.display(),
            err
        ))
    })?;

    let strip = env::var("STRIP").unwrap_or_else(|_| "strip".to_string());
    if opts.release
        && Command::new(&strip)
            .arg("--version")
            .stdout(Stdio::null())
            .status()
            .inspect_err(|_| eprintln!("checking for `strip` utility"))
            .is_ok()
    {
        println!("stripping the binary");
        let status = Command::new(&strip).arg(&dist_file).status()?;
        if !status.success() {
            Err("strip failed")?;
        }
    }

    Ok(())
}

fn lx(lx_args: Vec<String>, opts: &BuildOpts) -> Result<(), DynError> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut args = vec![
        "--locked".into(),
        "run".into(),
        "--no-default-features".into(),
    ];

    push_features(&mut args, opts);

    args.push("--bin".into());
    args.push("lx".into());
    args.push("--".into());

    args = args.into_iter().chain(lx_args).collect::<Vec<_>>();

    let mut command = Command::new(cargo);
    command.current_dir(project_root()).args(args);
    apply_cross_env(&mut command);
    static_gpg::apply(&mut command)?;
    let status = command.status()?;

    if !status.success() {
        Err("cargo run failed")?;
    }

    Ok(())
}

fn dist_man(opts: &BuildOpts) -> Result<(), DynError> {
    lx(
        vec![
            "util".into(),
            "man".into(),
            "--target-dir".into(),
            dist_dir().to_string_lossy().to_string(),
        ],
        opts,
    )
}

fn dist_completions(opts: &BuildOpts) -> Result<(), DynError> {
    lx(
        vec![
            "util".into(),
            "completion".into(),
            "--target-dir".into(),
            dist_dir().to_string_lossy().to_string(),
        ],
        opts,
    )
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

fn dist_package(opts: BuildOpts) -> Result<(), DynError> {
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
            opts.release,
            Some(xtask_lua::DistOpts {
                lua_feature: Some(lua_feature),
                clean_dist_dir: false,
                vendored: opts.vendored,
            }),
        )?;
    }
    println!("building lux-cli...");
    build(&opts)?;
    let target = build_target();
    let is_freebsd = target
        .as_deref()
        .is_some_and(|target| target.contains("-freebsd"));
    let tarball_target = target.filter(|target| target.contains("-musl") || is_freebsd);
    if !is_freebsd {
        println!("building man pages...");
        dist_man(&opts)?;
        println!("building shell completions...");
        dist_completions(&opts)?;
    }
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

    if !lx_bin_path.is_file() {
        Err(format!("{} not found", lx_bin_path.display()))?;
    }
    if let Some(target) = tarball_target {
        return dist_tarball(
            &dist_dir,
            &lx_bin_path,
            &manifest.workspace.package.version,
            &target,
            opts.release,
            &signing_config,
        );
    }
    dist_native(
        &dist_dir,
        &lx_bin_path,
        &manifest.workspace.package.version,
        opts.release,
        &signing_config,
    )
}

fn dist_native(
    dist_dir: &Path,
    lx_bin_path: &Path,
    version: &str,
    release: bool,
    signing_config: &SigningConfig,
) -> Result<(), DynError> {
    let project_root = project_root();
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
        .version(version)
        .out_dir(dist_dir)
        .binaries(vec![Binary::new(lx_bin_path).main(true)])
        .resources(resources)
        .description("A luxurious package manager for Lua")
        .homepage("https://lux.lumen-labs.org")
        .authors(vec!["mrcjkb", "vhyrro"])
        .publisher("lumen-oss")
        .identifier("org.lumenlabs.lux")
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
    if release {
        cargo_packager::package_and_sign(config_builder.config(), signing_config)
            .inspect_err(|err| eprintln!("failed to package lux:\n{err:?}"))?;
    } else {
        // GitHub Actions doesn't expose secrets on PR, so we don't sign the dry-run build.
        cargo_packager::package(config_builder.config())
            .inspect_err(|err| eprintln!("failed to package lux:\n{err:?}"))?;
    }
    Ok(())
}

fn dist_tarball(
    dist_dir: &Path,
    lx_bin_path: &Path,
    version: &str,
    target: &str,
    release: bool,
    signing_config: &SigningConfig,
) -> Result<(), DynError> {
    let tarball_path = dist_dir.join(format!("lux-{version}-{target}.tar.gz"));
    println!("building {}", tarball_path.display());
    let file = fs::File::create(&tarball_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.append_path_with_name(lx_bin_path, "lx")?;
    for (src, dest) in [
        (dist_dir.join("share/lux-lua"), "share/lux-lua"),
        (dist_dir.join("lib/pkgconfig"), "lib/pkgconfig"),
    ] {
        if src.is_dir() {
            builder.append_dir_all(dest, &src)?;
        } else {
            Err(format!("{} missing", src.display()))?;
        }
    }
    if !target.contains("-freebsd") {
        for (src, dest) in [
            (dist_dir.join("lx.1"), "lx.1"),
            (dist_dir.join("_lx"), "completions/zsh/_lx"),
            (dist_dir.join("lx.bash"), "completions/lx.bash"),
            (dist_dir.join("lx.fish"), "completions/lx.fish"),
            (dist_dir.join("lx.elv"), "completions/lx.elv"),
        ] {
            if src.is_file() {
                builder.append_path_with_name(&src, dest)?;
            } else {
                Err(format!("{} missing", src.display()))?;
            }
        }
    }
    builder.into_inner()?.finish()?;
    if release {
        cargo_packager::sign::sign_file(signing_config, &tarball_path)?;
    }
    Ok(())
}

fn gen_definitions() -> Result<(), DynError> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output_dir = project_root().join("definitions");
    let status = Command::new(cargo)
        .current_dir(project_root())
        .args([
            "run",
            "--package",
            "lux-definitions",
            "--",
            output_dir.to_string_lossy().as_ref(),
        ])
        .status()?;
    if !status.success() {
        Err("lux-definitions failed")?;
    }
    Ok(())
}

fn build_target() -> Option<String> {
    env::var("CARGO_BUILD_TARGET").ok()
}

fn cross_target() -> Option<String> {
    build_target().filter(|target| target != &Triple::host().to_string())
}

fn apply_cross_env(command: &mut Command) {
    if cross_target().is_some() {
        command.env("PKG_CONFIG_ALLOW_CROSS", "1");
        command.env("LIBZ_SYS_STATIC", "1");
    }
}

fn project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut project_root = Path::new(&manifest_dir)
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf();
    if project_root.is_dir() {
        let md = fs::metadata(&project_root).expect("failed to get manifest directory metadata");
        let permissions = md.permissions();
        if permissions.readonly() {
            // Probably in a nix build
            project_root = std::env::current_dir().expect("error getting current directory");
        }
    } else {
        project_root = std::env::current_dir().expect("error getting current directory");
    }
    project_root
}

fn dist_dir() -> PathBuf {
    project_root().join("target/dist")
}
