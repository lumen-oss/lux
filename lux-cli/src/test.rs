use clap::Args;
use eyre::Result;
use lux_lib::{
    config::Config,
    operations::{self, TestEnv},
    package::PackageName,
    workspace::Workspace,
};

#[derive(Args)]
pub struct Test {
    /// Extra arguments to pass to the test runner or test script.
    test_args: Option<Vec<String>>,

    /// Don't isolate the user environment (keep `HOME` and `XDG` environment variables).
    #[arg(long)]
    impure: bool,

    /// Ignore the project's lockfile and don't create one.
    #[arg(long)]
    no_lock: bool,

    /// Package to run tests for.
    #[arg(short, long, visible_short_alias = 'p')]
    package: Option<PackageName>,
}

pub async fn test(test: Test, config: Config) -> Result<()> {
    let workspace = Workspace::current_or_err()?;
    let test_args = test.test_args.unwrap_or_default();
    let test_env = if test.impure {
        TestEnv::Impure
    } else {
        TestEnv::Pure
    };
    operations::Test::new(workspace, &config)
        .args(test_args)
        .env(test_env)
        .no_lock(test.no_lock)
        .maybe_package(test.package)
        .run()
        .await?;
    Ok(())
}
