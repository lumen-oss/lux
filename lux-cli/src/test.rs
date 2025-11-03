use clap::Args;
use eyre::{OptionExt, Result};
use lux_lib::{
    config::Config,
    operations::{self, TestEnv},
    project::Project,
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
}

pub async fn test(test: Test, config: Config) -> Result<()> {
    let project = Project::current()?.ok_or_eyre("'lx test' must be run in a Lux project root.")?;
    let test_args = test.test_args.unwrap_or_default();
    let test_env = if test.impure {
        TestEnv::Impure
    } else {
        TestEnv::Pure
    };
    operations::Test::new(project, &config)
        .args(test_args)
        .env(test_env)
        .no_lock(test.no_lock)
        .run()
        .await?;
    Ok(())
}
