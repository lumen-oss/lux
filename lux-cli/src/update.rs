use clap::Args;
use eyre::{eyre, Context, OptionExt, Result};
use itertools::Itertools;
use lux_lib::package::{PackageName, PackageReq};
use lux_lib::progress::{MultiProgress, ProgressBar};
use lux_lib::project::Project;
use lux_lib::remote_package_db::PackageDB;
use lux_lib::rockspec::lua_dependency;
use lux_lib::{config::Config, operations};

#[derive(Args)]
pub struct Update {
    /// Skip the integrity checks for installed rocks when syncing the project lockfile.
    #[arg(long)]
    no_integrity_check: bool,

    /// Upgrade packages in the project's lux.toml (if operating on a project)
    #[arg(long)]
    toml: bool,

    /// Packages to update.
    /// When used with the --toml flag in a project, these must be package names.
    packages: Option<Vec<PackageReq>>,

    /// Build dependencies to update.
    /// Also called `dev`.
    /// When used with the --toml flag in a project, these must be package names.
    #[arg(short, long, alias = "dev", visible_short_aliases = ['d', 'b'])]
    build: Option<Vec<PackageReq>>,

    /// Build dependencies to update.
    /// When used with the --toml flag in a project, these must be package names.
    #[arg(short, long)]
    test: Option<Vec<PackageReq>>,
}

pub async fn update(args: Update, config: Config) -> Result<()> {
    let progress = MultiProgress::new_arc(&config);
    progress.map(|p| p.add(ProgressBar::from("🔎 Looking for updates...".to_string())));

    if args.toml {
        let mut project = Project::current()?.ok_or_eyre("No project found")?;

        let progress = MultiProgress::new(&config);
        let bar = progress.map(|progress| progress.new_bar());
        let db = PackageDB::from_config(&config, &bar).await?;
        let package_names = to_package_names(args.packages.as_ref())?;
        let mut upgrade_all = true;
        if let Some(packages) = package_names {
            upgrade_all = false;
            project
                .upgrade(lua_dependency::LuaDependencyType::Regular(packages), &db)
                .await?;
        }
        let build_package_names = to_package_names(args.build.as_ref())?;
        if let Some(packages) = build_package_names {
            upgrade_all = false;
            project
                .upgrade(lua_dependency::LuaDependencyType::Build(packages), &db)
                .await?;
        }
        let test_package_names = to_package_names(args.test.as_ref())?;
        if let Some(packages) = test_package_names {
            upgrade_all = false;
            project
                .upgrade(lua_dependency::LuaDependencyType::Test(packages), &db)
                .await?;
        }
        if upgrade_all {
            project.upgrade_all(&db).await?;
        }
    }

    let updated_packages = operations::Update::new(&config)
        .progress(progress)
        .packages(args.packages)
        .build_dependencies(args.build)
        .test_dependencies(args.test)
        .validate_integrity(!args.no_integrity_check)
        .update()
        .await
        .wrap_err("update failed.")?;

    if updated_packages.is_empty() {
        println!("Nothing to update.");
        return Ok(());
    }

    Ok(())
}

fn to_package_names(packages: Option<&Vec<PackageReq>>) -> Result<Option<Vec<PackageName>>> {
    if packages.is_some_and(|pkgs| !pkgs.iter().any(|pkg| pkg.version_req().is_any())) {
        return Err(eyre!(
            "Cannot use version constraints to upgrade dependencies in lux.toml."
        ));
    }
    Ok(packages
        .as_ref()
        .map(|pkgs| pkgs.iter().map(|pkg| pkg.name()).cloned().collect_vec()))
}
