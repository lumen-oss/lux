use clap::Args;
use eyre::{eyre, Context, Result};
use lux_lib::{
    config::Config,
    lua_rockspec::RemoteLuaRockspec,
    operations::{self, VendorTarget},
    project::Project,
};
use std::path::PathBuf;

#[derive(Args)]
pub struct Vendor {
    /// The directory in which to vendor the dependencies.
    vendor_dir: PathBuf,

    /// RockSpec to vendor the packages for.{n}
    /// If not set, Lux will vendor dependencies of the current project.
    #[arg(long)]
    rockspec: Option<PathBuf>,

    /// Ignore the project's lockfile and don't create one.
    #[arg(long)]
    no_lock: bool,

    /// Don't delete the `<vendor-dir>` when vendoring,{n}
    /// but rather keep all existing contents of the vendor directory.
    #[arg(long)]
    no_delete: bool,
}

pub async fn vendor(data: Vendor, config: Config) -> Result<()> {
    let target =
        match data.rockspec {
            Some(rockspec_path) => {
                let content = tokio::fs::read_to_string(&rockspec_path).await?;
                let rockspec = match rockspec_path
                    .extension()
                    .map(|ext| ext.to_string_lossy().to_string())
                    .unwrap_or("".into())
                    .as_str()
                {
                    "rockspec" => Ok(RemoteLuaRockspec::new(&content)?),
                    _ => Err(eyre!(
                        "expected a path to a .rockspec file, but got:\n{}",
                        rockspec_path.display()
                    )),
                }?;
                VendorTarget::Rockspec(rockspec)
            }
            None => VendorTarget::Project(Project::current_or_err().context(
                "`lx vendor` must be run in a project root or with a rockspec argument.",
            )?),
        };

    operations::Vendor::new()
        .vendor_dir(data.vendor_dir)
        .no_lock(data.no_lock)
        .no_delete(data.no_delete)
        .config(&config)
        .target(target)
        .vendor_dependencies()
        .await?;

    Ok(())
}
