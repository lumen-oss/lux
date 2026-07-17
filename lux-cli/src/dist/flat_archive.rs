use std::{
    io,
    path::{Path, PathBuf},
};

use clap::Args;
use lux_lib::{
    build::{Build, BuildBehaviour},
    config::{Config, ConfigBuilder},
    lockfile::LocalPackage,
    lua_installation::LuaInstallation,
    lua_rockspec::RemoteLuaRockspec,
    lua_version::LuaVersion,
    operations::{Install, InstallProject, PackageInstallSpec},
    package::{PackageName, PackageReq},
    tree::{self, FlatDistTree, InstallTree},
    workspace::Workspace,
};

use miette::{miette, IntoDiagnostic, Result, WrapErr};
use path_slash::PathExt;
use tempfile::{tempdir, TempDir};
use tokio::fs::{self, File};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, ZipWriter};

use crate::{
    args::{OutputFormat, PackageOrRockspec},
    workspace::exists_matching_workspace_member,
};

#[derive(Args)]
pub struct FlatArchive {
    /// Path to a RockSpec or a package query for a package to distribute.{n}
    /// Prioritises local projects if in a workspace, then installed rocks.{n}
    /// If there is no matching workspace member or installed rock,{n}
    /// a rock will be downloaded and installed to a temporary directory.{n}
    /// In case of multiple matches, the latest version will be distributed.{n}
    ///{n}
    /// Examples:{n}
    ///     - "pkg"{n}
    ///     - "pkg@1.0.0"{n}
    ///     - "pkg>=1.0.0"{n}
    ///     - "/path/to/foo-1.0.0-1.rockspec"{n}
    ///{n}
    /// If not set, lux will attempt to distribute the current project.{n}
    /// Must be set in multi-project workspaces.
    #[clap(value_parser)]
    package_or_rockspec: Option<PackageOrRockspec>,

    /// The destination path. Defaults to '<cwd>/<package>-<version>.zip'.{n}
    #[arg(short, long, visible_short_alias = 'd')]
    destination: Option<PathBuf>,

    #[clap(default_value_t=CompressionMethod::default())]
    #[arg(short, long, value_enum, visible_short_alias = 'c')]
    compression_method: CompressionMethod,

    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

#[derive(Clone, clap::ValueEnum, Default)]
enum CompressionMethod {
    /// Store the install tree as is
    #[default]
    Stored,
    /// Compress the install tree using Deflate
    Deflated,
    /// Compress the install tree using BZIP2
    Bzip2,
    /// Compress the install tree using XZ
    Xz,
    /// Compress the install tree using `ZStandard`
    Zstd,
    /// Compress the install tree using LZMA
    Lzma,
}

pub async fn dist_archive(args: FlatArchive, config: Config) -> Result<()> {
    let staging_dir = tempdir().into_diagnostic()?;
    let config = ConfigBuilder::from(config)
        // Wrapping bin scripts does not make sense for distributed packages.
        .wrap_bin_scripts(Some(false))
        .user_tree(Some(staging_dir.path().to_path_buf()))
        .build()?;

    let (pkg, install_root) = match &args.package_or_rockspec {
        None => install_project(None, &staging_dir, &config).await,
        Some(PackageOrRockspec::Package(package_req))
            if exists_matching_workspace_member(package_req)? =>
        {
            install_project(Some(package_req.name()), &staging_dir, &config).await
        }
        Some(PackageOrRockspec::Package(package)) => {
            install_package(package, &staging_dir, &config).await
        }
        Some(PackageOrRockspec::RockSpec(rockspec_path)) => {
            install_rockspec(rockspec_path, &staging_dir, &config).await
        }
    }?;

    let destination = args
        .destination
        .clone()
        .map(|dest| {
            if dest.is_dir() {
                dest.join(format!("{}-{}.zip", pkg.name(), pkg.version()))
            } else {
                dest
            }
        })
        .unwrap_or(PathBuf::from(format!(
            "{}-{}.zip",
            pkg.name(),
            pkg.version()
        )));

    zip_dir(&install_root, &destination, &args.compression_method).await?;

    match args.output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(&destination).into_diagnostic()?)
        }
        OutputFormat::Text => println!("Wrote archive to {}", destination.display()),
    }

    Ok(())
}

async fn install_project(
    package: Option<&PackageName>,
    staging_dir: &TempDir,
    config: &Config,
) -> Result<(LocalPackage, PathBuf)> {
    let workspace = Workspace::current_or_err()?;
    let project = match package {
        Some(package) => workspace.select_member(package)?,
        None => workspace.single_member()?,
    };
    let lua_version = project.lua_version(config)?;
    let tree = FlatDistTree::new(staging_dir.path().to_path_buf(), lua_version, config)?;
    Ok((
        InstallProject::new()
            .project(project)
            .config(config)
            .tree(&tree)
            .build()
            .await?,
        tree.root(),
    ))
}

async fn install_package(
    package: &PackageReq,
    staging_dir: &TempDir,
    config: &Config,
) -> Result<(LocalPackage, PathBuf)> {
    let lua_version = LuaVersion::from(config)?.clone();
    let tree = FlatDistTree::new(staging_dir.path().to_path_buf(), lua_version, config)?;
    let packages = Install::new(config)
        .package(
            PackageInstallSpec::new(package.clone(), tree::EntryType::Entrypoint)
                .build_behaviour(BuildBehaviour::Force)
                .build(),
        )
        .tree(tree.clone())
        .install()
        .await?;
    let package = packages
        .into_iter()
        .find(|pkg| pkg.name() == package.name())
        .ok_or_else(|| miette!("package was not installed"))?;
    Ok((package, tree.root()))
}

async fn install_rockspec(
    rockspec_path: &Path,
    staging_dir: &TempDir,
    config: &Config,
) -> Result<(LocalPackage, PathBuf)> {
    let content = tokio::fs::read_to_string(&rockspec_path)
        .await
        .into_diagnostic()?;
    let lua_version = LuaVersion::from(config)?.clone();
    let rockspec = match rockspec_path
        .extension()
        .map(|ext| ext.to_string_lossy().to_string())
        .unwrap_or("".into())
        .as_str()
    {
        "rockspec" => Ok(RemoteLuaRockspec::new(&content)?),
        _ => Err(miette!(
            "expected a path to a .rockspec or a package requirement."
        )),
    }?;
    let lua = LuaInstallation::new(
        &lua_version,
        config,
    )
    .await?;
    let tree = FlatDistTree::new(staging_dir.path().to_path_buf(), lua_version, config)?;
    let package = Build::new()
        .rockspec(&rockspec)
        .lua(&lua)
        .tree(&tree)
        .entry_type(tree::EntryType::Entrypoint)
        .config(config)
        .build()
        .await?;
    Ok((package, tree.root()))
}

async fn zip_dir(src_dir: &Path, dest_file: &Path, method: &CompressionMethod) -> Result<()> {
    if dest_file.exists() {
        return Err(miette!("File {} already exists!", dest_file.display()));
    }
    let temp_archive = PathBuf::from(format!("{}.part", dest_file.display()));
    let archive = File::create(&temp_archive)
        .await
        .into_diagnostic()?
        .into_std()
        .await;
    let walkdir = WalkDir::new(src_dir);
    let mut zip = ZipWriter::new(archive);

    let compression_method = match method {
        CompressionMethod::Stored => zip::CompressionMethod::Stored,
        CompressionMethod::Deflated => zip::CompressionMethod::Deflated,
        CompressionMethod::Bzip2 => zip::CompressionMethod::Bzip2,
        CompressionMethod::Xz => zip::CompressionMethod::Xz,
        CompressionMethod::Zstd => zip::CompressionMethod::Zstd,
        CompressionMethod::Lzma => zip::CompressionMethod::Lzma,
    };

    #[cfg(target_family = "unix")]
    let options = SimpleFileOptions::default()
        .compression_method(compression_method)
        .unix_permissions(0o755);

    #[cfg(target_family = "windows")]
    let options = SimpleFileOptions::default().compression_method(compression_method);

    for entry_result in walkdir.into_iter() {
        let entry = entry_result.map_err(|err| {
            miette!(
                "Error while traversing directory {}: {}.",
                src_dir.display(),
                err,
            )
        })?;
        let path = entry.path();
        let relative_path = path.strip_prefix(src_dir).into_diagnostic()?;
        let relative_path_str = relative_path.to_slash_lossy().to_string();
        if path.is_file() {
            zip.start_file(relative_path_str, options)
                .into_diagnostic()?;
            let mut f = File::open(path).await.into_diagnostic()?.into_std().await;
            io::copy(&mut f, &mut zip).into_diagnostic()?;
        } else if !relative_path.as_os_str().is_empty() {
            zip.add_directory(relative_path_str, options)
                .into_diagnostic()?;
        }
    }
    zip.finish().into_diagnostic()?;
    fs::rename(&temp_archive, &dest_file)
        .await
        .into_diagnostic()
        .wrap_err(format!(
            "Error renaming {} to {}",
            temp_archive.display(),
            dest_file.display()
        ))?;
    Ok(())
}
