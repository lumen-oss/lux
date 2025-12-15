use eyre::{OptionExt, Result};
use itertools::{Either, Itertools};
use lux_lib::{
    config::Config,
    progress::MultiProgress,
    project::Project,
    remote_package_db::PackageDB,
    rockspec::lua_dependency::{self},
};

use crate::utils::project::{
    sync_build_dependencies_if_locked, sync_dependencies_if_locked,
    sync_test_dependencies_if_locked, PackageReqOrGitShorthand,
};

#[derive(clap::Args)]
pub struct Add {
    /// Package or list of packages to install and add to the project's dependencies. {n}
    /// Examples: "pkg", "pkg@1.0.0", "pkg>=1.0.0" {n}
    /// If you do not specify a version requirement, lux will fetch the latest version. {n}
    /// {n}
    /// You can also specify git packages by providing a git URL shorthand. {n}
    /// Example: "github:owner/repo" {n}
    /// Supported git host prefixes are: "github:", "gitlab:", "sourcehut:" and "codeberg:". {n}
    /// Lux will automatically fetch the latest SemVer tag or commit SHA if no SemVer tag is found. {n}
    /// Note that projects with git dependencies cannot be published to luarocks.org.
    package_req: Vec<PackageReqOrGitShorthand>,

    /// Reinstall without prompt if a package is already installed.
    #[arg(long, visible_short_alias = 'f')]
    force: bool,

    /// Install the package as a development dependency. {n}
    /// Also called `dev`.
    #[arg(short, long, alias = "dev", visible_short_aliases = ['d', 'b'])]
    build: Option<Vec<PackageReqOrGitShorthand>>,

    /// Install the package as a test dependency.
    #[arg(short, long, visible_short_alias = 't')]
    test: Option<Vec<PackageReqOrGitShorthand>>,
}

pub async fn add(data: Add, config: Config) -> Result<()> {
    let mut project = Project::current()?.ok_or_eyre("No project found")?;

    let progress = MultiProgress::new(&config);
    let bar = progress.map(MultiProgress::new_bar);
    let db = PackageDB::from_config(&config, &bar).await?;

    let progress = MultiProgress::new_arc(&config);

    let (dependencies, git_dependencies): (Vec<_>, Vec<_>) =
        data.package_req.iter().partition_map(|req| match req {
            PackageReqOrGitShorthand::PackageReq(req) => Either::Left(req.clone()),
            PackageReqOrGitShorthand::GitShorthand(url) => Either::Right(url.clone()),
        });

    if !data.package_req.is_empty() {
        project
            .add(lua_dependency::DependencyType::Regular(dependencies), &db)
            .await?;
        project
            .add_git(lua_dependency::LuaDependencyType::Regular(git_dependencies))
            .await?;
        sync_dependencies_if_locked(&project, progress.clone(), &config).await?;
    }

    let build_packages = data.build.unwrap_or_default();
    if !build_packages.is_empty() {
        let (dependencies, git_dependencies): (Vec<_>, Vec<_>) =
            build_packages.iter().partition_map(|req| match req {
                PackageReqOrGitShorthand::PackageReq(req) => Either::Left(req.clone()),
                PackageReqOrGitShorthand::GitShorthand(url) => Either::Right(url.clone()),
            });
        project
            .add(lua_dependency::DependencyType::Build(dependencies), &db)
            .await?;
        project
            .add_git(lua_dependency::LuaDependencyType::Build(git_dependencies))
            .await?;
        sync_build_dependencies_if_locked(&project, progress.clone(), &config).await?;
    }

    let test_packages = data.test.unwrap_or_default();
    if !test_packages.is_empty() {
        let (dependencies, git_dependencies): (Vec<_>, Vec<_>) =
            test_packages.iter().partition_map(|req| match req {
                PackageReqOrGitShorthand::PackageReq(req) => Either::Left(req.clone()),
                PackageReqOrGitShorthand::GitShorthand(url) => Either::Right(url.clone()),
            });
        project
            .add(lua_dependency::DependencyType::Test(dependencies), &db)
            .await?;
        project
            .add_git(lua_dependency::LuaDependencyType::Test(git_dependencies))
            .await?;
        sync_test_dependencies_if_locked(&project, progress.clone(), &config).await?;
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use assert_fs::{prelude::PathCopy, TempDir};
    use lux_lib::config::ConfigBuilder;
    use serial_test::serial;

    use super::*;
    use std::path::PathBuf;

    #[serial]
    #[tokio::test]
    async fn test_add_regular_dependencies() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let sample_project: PathBuf = "resources/test/sample-projects/init/".into();
        let project_root = TempDir::new().unwrap();
        project_root.copy_from(&sample_project, &["**"]).unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project_root).unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let args = Add {
            package_req: vec!["penlight@1.5".parse().unwrap()],
            force: false,
            build: Option::None,
            test: Option::None,
        };
        add(args, config.clone()).await.unwrap();
        let lockfile_path = project_root.join("lux.lock");
        let lockfile_content =
            String::from_utf8(tokio::fs::read(&lockfile_path).await.unwrap()).unwrap();
        assert!(lockfile_content.contains("penlight"));
        assert!(lockfile_content.contains("luafilesystem")); // dependency

        let args = Add {
            package_req: vec!["md5".parse().unwrap()],
            force: false,
            build: Option::None,
            test: Option::None,
        };
        add(args, config.clone()).await.unwrap();
        let lockfile_path = project_root.join("lux.lock");
        let lockfile_content =
            String::from_utf8(tokio::fs::read(&lockfile_path).await.unwrap()).unwrap();
        assert!(lockfile_content.contains("penlight"));
        assert!(lockfile_content.contains("luafilesystem"));
        assert!(lockfile_content.contains("md5"));

        std::env::set_current_dir(&cwd).unwrap();
    }

    #[serial]
    #[tokio::test]
    async fn test_add_build_dependencies() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let sample_project: PathBuf = "resources/test/sample-projects/init/".into();
        let project_root = TempDir::new().unwrap();
        project_root.copy_from(&sample_project, &["**"]).unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project_root).unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let args = Add {
            package_req: Vec::new(),
            force: false,
            build: Option::Some(vec!["penlight@1.5".parse().unwrap()]),
            test: Option::None,
        };
        add(args, config.clone()).await.unwrap();
        let lockfile_path = project_root.join("lux.lock");
        let lockfile_content =
            String::from_utf8(tokio::fs::read(&lockfile_path).await.unwrap()).unwrap();
        assert!(lockfile_content.contains("penlight"));
        assert!(lockfile_content.contains("luafilesystem")); // dependency

        let args = Add {
            package_req: Vec::new(),
            force: false,
            build: Option::Some(vec!["md5".parse().unwrap()]),
            test: Option::None,
        };
        add(args, config.clone()).await.unwrap();
        let lockfile_path = project_root.join("lux.lock");
        let lockfile_content =
            String::from_utf8(tokio::fs::read(&lockfile_path).await.unwrap()).unwrap();
        assert!(lockfile_content.contains("penlight"));
        assert!(lockfile_content.contains("luafilesystem"));
        assert!(lockfile_content.contains("md5"));

        std::env::set_current_dir(&cwd).unwrap();
    }

    #[serial]
    #[tokio::test]
    async fn test_add_test_dependencies() {
        if std::env::var("LUX_SKIP_IMPURE_TESTS").unwrap_or("0".into()) == "1" {
            println!("Skipping impure test");
            return;
        }
        let sample_project: PathBuf = "resources/test/sample-projects/init/".into();
        let project_root = TempDir::new().unwrap();
        project_root.copy_from(&sample_project, &["**"]).unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project_root).unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let args = Add {
            package_req: Vec::new(),
            force: false,
            build: Option::None,
            test: Option::Some(vec!["penlight@1.5".parse().unwrap()]),
        };
        add(args, config.clone()).await.unwrap();
        let lockfile_path = project_root.join("lux.lock");
        let lockfile_content =
            String::from_utf8(tokio::fs::read(&lockfile_path).await.unwrap()).unwrap();
        assert!(lockfile_content.contains("penlight"));
        assert!(lockfile_content.contains("luafilesystem")); // dependency

        let args = Add {
            package_req: Vec::new(),
            force: false,
            build: Option::None,
            test: Option::Some(vec!["md5".parse().unwrap()]),
        };
        add(args, config.clone()).await.unwrap();
        let lockfile_path = project_root.join("lux.lock");
        let lockfile_content =
            String::from_utf8(tokio::fs::read(&lockfile_path).await.unwrap()).unwrap();
        assert!(lockfile_content.contains("penlight"));
        assert!(lockfile_content.contains("luafilesystem"));
        assert!(lockfile_content.contains("md5"));

        std::env::set_current_dir(&cwd).unwrap();
    }
}
