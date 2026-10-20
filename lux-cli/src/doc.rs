use clap::Args;
use inquire::{Confirm, Select};
use itertools::Itertools;
use lux_lib::{
    config::Config,
    lockfile::{LockedPackage, OptState, PinnedState},
    lua_rockspec::RemoteLuaRockspec,
    package::PackageReq,
    rockspec::Rockspec,
    tree::{InstallTree, RockMatches, Tree},
};
use miette::{miette, Context, IntoDiagnostic, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use url::Url;
use walkdir::WalkDir;

use crate::workspace::current_workspace_or_user_tree;

#[derive(Args)]
pub struct Doc {
    /// The package to show documentation for.{n}
    /// If not set, lux will use the package installed in the tree,{n}
    /// or prompt to select one if the tree has many.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageReq>,

    /// Ignore local docs and open the package's homepage in a browser.
    #[arg(long)]
    online: bool,
}

fn package_label(pkg: &LockedPackage) -> String {
    let mut modifiers = Vec::new();
    if pkg.pinned() == PinnedState::Pinned {
        modifiers.push("pinned".to_string());
    }
    if pkg.opt() == OptState::Optional {
        modifiers.push("optional".to_string());
    }
    if let Some(constraint) = pkg.spec().constraint().to_string_opt() {
        modifiers.push(format!("constraint: {constraint}"));
    }
    if modifiers.is_empty() {
        format!("{}@{}", pkg.name(), pkg.version())
    } else {
        format!(
            "{}@{} ({})",
            pkg.name(),
            pkg.version(),
            modifiers.join(", ")
        )
    }
}

pub async fn doc(args: Doc, config: Config) -> Result<()> {
    let tree = current_workspace_or_user_tree(&config)?;
    let package_req = match &args.package {
        Some(package_req) => package_req.clone(),
        None => {
            let candidates = tree
                .list()?
                .into_values()
                .flatten()
                .map(|pkg| pkg.into_package_spec().into_package_req().to_string())
                .unique()
                .sorted()
                .collect_vec();
            let selected = match candidates.as_slice() {
                [] => {
                    return Err(miette!(
                        help = "install a package into this tree, or specify a package with '--package=<PACKAGE>'",
                        "no packages found in this tree"
                    ));
                }
                [candidate] => candidate.clone(),
                candidates => {
                    if config.no_prompt() {
                        return Err(miette!(
                            help = "specify the package with '--package=<PACKAGE>'",
                            "multiple packages found in this tree, but no package specified"
                        ));
                    }
                    Select::new(
                        "Please select a package to show documentation for.",
                        candidates.to_vec(),
                    )
                    .prompt()
                    .into_diagnostic()
                    .wrap_err("error selecting from multiple packages")?
                }
            };
            PackageReq::parse(&selected)?
        }
    };
    let lockfile = tree.lockfile()?;
    let package_id = match tree.match_rocks(&package_req)? {
        RockMatches::NotFound(package_req) => {
            Err(miette!("no package matching {package_req} found"))
        }
        RockMatches::Many(package_ids) => {
            if config.no_prompt() {
                Err(miette!(
                    help = "specify an exact package (<name>@<version>) or narrow the version requirement",
                    "found multiple packages matching {}",
                    &args.package
                ))
            } else {
                let candidates: Vec<_> = package_ids
                    .into_iter()
                    .filter_map(|id| lockfile.get(&id).map(|pkg| (id, pkg.clone())))
                    .collect();
                let labels = candidates
                    .iter()
                    .map(|(_, pkg)| package_label(pkg))
                    .collect_vec();
                let selected = Select::new(
                    &format!(
                        "Multiple packages matching {} found. Please select one to open.",
                        &args.package
                    ),
                    labels,
                )
                .raw_prompt()
                .into_diagnostic()
                .wrap_err("error selecting from multiple packages")?;
                Ok(candidates[selected.index].0.clone())
            }
        }
        RockMatches::Single(package_id) => Ok(package_id),
    }?;
    let pkg = lockfile
        .get(&package_id)
        .ok_or_else(|| miette!("package is installed, but not found in the lockfile"))?
        .clone();
    if args.online {
        open_homepage(pkg, &tree).await
    } else {
        open_local_docs(pkg, &tree, &config).await
    }
}

async fn open_homepage(pkg: LockedPackage, tree: &Tree) -> Result<()> {
    let homepage = match get_homepage(&pkg, tree)? {
        Some(homepage) => Ok(homepage),
        None => Err(miette!(
            "package {} does not have a homepage in its RockSpec",
            pkg.into_package_spec()
        )),
    }?;
    open::that(homepage.to_string()).into_diagnostic()?;
    Ok(())
}

fn get_homepage(pkg: &LockedPackage, tree: &Tree) -> Result<Option<Url>> {
    let rockspec_content =
        std::fs::read_to_string(tree.layout_for(pkg.spec()).rockspec_path()).into_diagnostic()?;
    let rockspec = RemoteLuaRockspec::new(&rockspec_content)?;
    Ok(rockspec.description().homepage.clone())
}

fn is_plain_text(path: &Path) -> Result<bool> {
    let mut buf = Vec::with_capacity(1024);
    std::fs::File::open(path)
        .into_diagnostic()?
        .take(1024)
        .read_to_end(&mut buf)
        .into_diagnostic()?;
    Ok(content_inspector::inspect(&buf).is_text())
}

fn open_doc_file(path: &Path) -> Result<()> {
    if is_plain_text(path).wrap_err_with(|| format!("error reading '{}'", path.display()))? {
        edit::edit_file(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not open '{}' for editing", path.display()))?;
    } else {
        open::that(path)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not open '{}' in a browser", path.display()))?;
    }
    Ok(())
}

async fn open_local_docs(pkg: LockedPackage, tree: &Tree, config: &Config) -> Result<()> {
    let doc_dir = tree.layout_for(pkg.spec()).doc;
    let files: Vec<PathBuf> = WalkDir::new(&doc_dir)
        .into_iter()
        .filter_map_ok(|file| {
            let path = file.into_path();
            path.is_file().then_some(path)
        })
        .try_collect()
        .into_diagnostic()?;
    match files.as_slice() {
        [file] => open_doc_file(file),
        [_, _, ..] => {
            let labels = files
                .iter()
                .map(|file| {
                    file.strip_prefix(&doc_dir)
                        .unwrap_or(file.as_path())
                        .to_string_lossy()
                        .to_string()
                })
                .collect_vec();
            let selected = Select::new(
                "Multiple documentation files found. Please select one to open.",
                labels,
            )
            .raw_prompt()
            .into_diagnostic()
            .wrap_err("error selecting from multiple files")?;
            open_doc_file(&files[selected.index])
        }
        [] => match get_homepage(&pkg, tree)? {
            None => Err(miette!(
                "no documentation found for package '{}'",
                pkg.into_package_spec()
            )),
            Some(homepage) => {
                if config.no_prompt() {
                    return Err(miette!(
                        "no local documentation found for package '{}'",
                        pkg.into_package_spec()
                    ));
                } else if Confirm::new("No local documentation found. Open homepage?")
                    .with_default(false)
                    .prompt()
                    .into_diagnostic()
                    .wrap_err("error prompting to open homepage")?
                {
                    open::that(homepage.to_string()).into_diagnostic()?;
                }
                Ok(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;
    use std::io::Write;

    #[test]
    fn detects_plain_text() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("README.md");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "# Hello, world!").unwrap();
        assert!(is_plain_text(&file).unwrap());
    }

    #[test]
    fn detects_binary() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("diagram.png");
        let mut f = std::fs::File::create(&file).unwrap();
        f.write_all(b"\x89PNG\r\n\x1a\n").unwrap();
        assert!(!is_plain_text(&file).unwrap());
    }
}
