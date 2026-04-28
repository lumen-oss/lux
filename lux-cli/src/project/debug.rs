use clap::Args;
use eyre::Result;
use lux_lib::workspace::Workspace;

use crate::utils::file_tree::term_tree_from_paths;

#[derive(Args)]
pub struct DebugProject {
    /// List files that are included.
    /// To avoid copying large files that are not relevant to the build process,
    /// Lux excludes hidden files and files that are ignored
    /// (e.g. by .gitignore or other .ignore files).
    #[arg(long)]
    list_files: bool,
}

pub fn debug_project(args: DebugProject) -> Result<()> {
    let workspace = Workspace::current()?;

    if let Some(workspace) = workspace {
        for project in workspace.members() {
            let toml = project.toml();

            println!("Project name: {}", toml.package());
            println!("Project version: {}", toml.version()?);

            println!("Project location: {}", project.root().display());

            if args.list_files {
                let project_files = project.project_files();
                if project_files.is_empty() {
                    println!("\nNo included project files detected.");
                } else {
                    let project_tree = term_tree_from_paths(&project_files);
                    println!("\nIncluded project files:\n\n{project_tree}.");
                }
            }
        }
    } else {
        eprintln!("Could not find project in current directory.");
    }

    Ok(())
}
