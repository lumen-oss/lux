use crate::utils::addons::{
    derive_implicit_addons, install_lls_addons_for_project, AddonInstallReport,
};
use clap::Args;
use eyre::{eyre, Context, OptionExt, Result};
use itertools::Itertools;
use lux_lib::package::{PackageName, PackageReq};
use lux_lib::progress::{MultiProgress, ProgressBar};
use lux_lib::project::Project;
use lux_lib::remote_package_db::RemotePackageDB;
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
        let db = RemotePackageDB::from_config(&config, &bar).await?;
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

    // After syncing/updating, install LuaLS addons if configured
    if let Some(project) = Project::current()? {
        if let Ok(local) = project.toml().into_local() {
            install_addons_for_project(&project, &config, &local).await?;
        }
    }

    if updated_packages.is_empty() {
        println!("Nothing to update.");
        return Ok(());
    }

    Ok(())
}

async fn install_addons_for_project(
    project: &Project,
    config: &Config,
    local: &lux_lib::project::project_toml::LocalProjectToml,
) -> Result<()> {
    // Explicit addons (strict): union across tiers
    let mut explicit: Vec<String> = Vec::new();
    explicit.extend(local.dependencies_addons().iter().cloned());
    explicit.extend(local.test_dependencies_addons().iter().cloned());
    explicit.extend(local.build_dependencies_addons().iter().cloned());
    explicit.sort();
    explicit.dedup();
    // Implicit addons (best-effort)
    let implicit: Vec<String> = derive_implicit_addons(local, project);

    let mut all_entries = Vec::new();
    if !explicit.is_empty() {
        let report = install_lls_addons_for_project(project, config, &explicit, true).await?;
        let has_failures = report.entries.iter().any(|e| !e.ok);
        if config.verbose() || has_failures {
            print_addons_report("Explicit addons", &report, config.verbose());
        }
        all_entries.extend(report.entries);
    }
    if local.check_dependencies() {
        // Install implicit ones, but do not fail the update if they are missing
        let implicit_filtered: Vec<String> = implicit
            .into_iter()
            .filter(|a| !explicit.iter().any(|e| e == a))
            .collect();
        if !implicit_filtered.is_empty() {
            if let Ok(report) =
                install_lls_addons_for_project(project, config, &implicit_filtered, false).await
            {
                // Only show implicit report in verbose mode (even on failures)
                if config.verbose() {
                    print_addons_report("Implicit addons", &report, config.verbose());
                }
                all_entries.extend(report.entries);
            }
        }
    }
    // Persist resolved addons snapshot to lux.lock
    if !all_entries.is_empty() {
        let lockfile = project.lockfile()?;
        let mut guard = lockfile.write_guard();
        let resolved = all_entries
            .iter()
            .filter(|e| e.ok)
            .map(|e| lux_lib::lockfile::ResolvedAddon {
                name: e.name.clone(),
                source: e.source.clone(),
                implicit: !e.required,
                library_paths: e.library_paths.clone(),
                version: e.version.clone(),
                commit: e.commit.clone(),
            })
            .collect::<Vec<_>>();
        guard.set_addons(resolved);
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

fn print_addons_report(title: &str, report: &AddonInstallReport, verbose: bool) {
    if report.entries.is_empty() {
        return;
    }
    println!("{title}:");
    for e in &report.entries {
        let status = if e.ok { "ok" } else { "failed" };
        let required = if e.required { "required" } else { "optional" };
        if verbose {
            let msg = e.message.as_deref().unwrap_or("");
            if msg.is_empty() {
                println!(
                    "  - {} [{} | {} via {}]",
                    e.name, required, status, e.source
                );
            } else {
                println!(
                    "  - {} [{} | {} via {}] {}",
                    e.name, required, status, e.source, msg
                );
            }
        } else {
            println!(
                "  - {} [{} | {} via {}]",
                e.name, required, status, e.source
            );
        }
    }
}
