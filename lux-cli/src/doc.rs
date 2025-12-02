use clap::Args;
use eyre::{eyre, Context, OptionExt, Result};
use inquire::{Confirm, Select};
use itertools::Itertools;
use lux_lib::{
    config::{Config, LuaVersion},
    lockfile::LocalPackage,
    lua_rockspec::RemoteLuaRockspec,
    package::PackageReq,
    rockspec::Rockspec,
    tree::{RockMatches, Tree},
};
use url::Url;
use walkdir::WalkDir;

#[derive(Args)]
pub struct Doc {
    package: PackageReq,

    /// Ignore local docs and open the package's homepage in a browser.
    #[arg(long)]
    online: bool,
}

pub async fn doc(args: Doc, config: Config) -> Result<()> {
    let tree = config.user_tree(LuaVersion::from(&config)?.clone())?;
    let package_id = match tree.match_rocks(&args.package)? {
        RockMatches::NotFound(package_req) => {
            Err(eyre!("No package matching {} found.", package_req))
        }
        RockMatches::Many(_package_ids) => Err(eyre!(
            "
Found multiple packages matching {}.
Please specify an exact package (<name>@<version>) or narrow the version requirement.
",
            &args.package
        )),
        RockMatches::Single(package_id) => Ok(package_id),
    }?;
    let lockfile = tree.lockfile()?;
    let pkg = lockfile
        .get(&package_id)
        .ok_or_eyre("package is installed, but not found in the lockfile")?
        .clone();
    if args.online {
        open_homepage(pkg, &tree).await
    } else {
        open_local_docs(pkg, &tree).await
    }
}

async fn open_homepage(pkg: LocalPackage, tree: &Tree) -> Result<()> {
    let homepage = match get_homepage(&pkg, tree)? {
        Some(homepage) => Ok(homepage),
        None => Err(eyre!(
            "Package {} does not have a homepage in its RockSpec.",
            pkg.into_package_spec()
        )),
    }?;
    open::that(homepage.to_string())?;
    Ok(())
}

fn get_homepage(pkg: &LocalPackage, tree: &Tree) -> Result<Option<Url>> {
    let layout = tree.installed_rock_layout(pkg)?;
    let rockspec_content = std::fs::read_to_string(layout.rockspec_path())?;
    let rockspec = RemoteLuaRockspec::new(&rockspec_content)?;
    Ok(rockspec.description().homepage.clone())
}

async fn open_local_docs(pkg: LocalPackage, tree: &Tree) -> Result<()> {
    let layout = tree.installed_rock_layout(&pkg)?;
    let files: Vec<String> = WalkDir::new(&layout.doc)
        .into_iter()
        .filter_map_ok(|file| {
            let path = file.into_path();
            if path.is_file() {
                path.file_name()
                    .map(|file_name| file_name.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .try_collect()?;
    match files.first() {
        Some(file) if files.len() == 1 => {
            edit::edit_file(layout.doc.join(file))?;
            Ok(())
        }
        Some(_) => {
            let file = Select::new(
                "Multiple documentation files found. Please select one to open.",
                files,
            )
            .prompt()
            .wrap_err("error selecting from multiple files")?;
            edit::edit_file(layout.doc.join(file))?;
            Ok(())
        }
        None => match get_homepage(&pkg, tree)? {
            None => Err(eyre!(
                "No documentation found for package {}",
                pkg.into_package_spec()
            )),
            Some(homepage) => {
                if Confirm::new("No local documentation found. Open homepage?")
                    .with_default(false)
                    .prompt()
                    .wrap_err("error prompting to open homepage")?
                {
                    open::that(homepage.to_string())?;
                }
                Ok(())
            }
        },
    }
}
