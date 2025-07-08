use std::time::Duration;

use clap::Parser;
use eyre::Result;
use lux_cli::{
    add, build, check, completion, config,
    debug::Debug,
    doc, download, exec, fetch, format, generate_rockspec, info, install, install_lua,
    install_rockspec, list, outdated, pack, path, pin, project, purge, remove, run, run_lua,
    search, shell, test, uninstall, unpack, update,
    upload::{self},
    which, Cli, Commands,
};
use lux_lib::{
    config::{tree::RockLayoutConfig, ConfigBuilder},
    lockfile::PinnedState::{Pinned, Unpinned},
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config_builder = ConfigBuilder::new()
        .unwrap()
        .dev(Some(cli.dev))
        .lua_dir(cli.lua_dir)
        .lua_version(cli.lua_version)
        .namespace(cli.namespace)
        .extra_servers(cli.extra_servers)
        .only_sources(cli.only_sources)
        .server(cli.server)
        .user_tree(cli.tree)
        .timeout(
            cli.timeout
                .map(|duration| Duration::from_secs(duration as u64)),
        )
        .no_project(Some(cli.no_project))
        .variables(
            cli.variables
                .map(|variables| variables.into_iter().collect()),
        )
        .verbose(Some(cli.verbose));

    if cli.nvim {
        config_builder = config_builder.entrypoint_layout(RockLayoutConfig::new_nvim_layout());
    }

    let config = config_builder.build()?;

    match cli.command {
        Commands::Completion(completion_args) => completion::completion(completion_args).await?,
        Commands::Search(search_data) => search::search(search_data, config).await?,
        Commands::Download(download_data) => download::download(download_data, config).await?,
        Commands::Debug(debug) => match debug {
            Debug::FetchRemote(unpack_data) => fetch::fetch_remote(unpack_data, config).await?,
            Debug::Unpack(unpack_data) => unpack::unpack(unpack_data).await?,
            Debug::UnpackRemote(unpack_data) => unpack::unpack_remote(unpack_data, config).await?,
            Debug::Project(debug_project) => project::debug_project(debug_project)?,
        },
        Commands::New(project_data) => project::write_project_rockspec(project_data).await?,
        Commands::Build(build_data) => {
            build::build(build_data, config).await?;
        }
        Commands::List(list_data) => list::list_installed(list_data, config)?,
        Commands::Lua(run_lua) => run_lua::run_lua(run_lua, config).await?,
        Commands::Install(install_data) => install::install(install_data, config).await?,
        Commands::InstallRockspec(install_data) => {
            install_rockspec::install_rockspec(install_data, config).await?
        }
        Commands::Outdated(outdated) => outdated::outdated(outdated, config).await?,
        Commands::InstallLua => install_lua::install_lua(config).await?,
        Commands::Fmt(fmt_args) => format::format(fmt_args)?,
        Commands::Purge => purge::purge(config).await?,
        Commands::Remove(remove_args) => remove::remove(remove_args, config).await?,
        Commands::Exec(run_args) => exec::exec(run_args, config).await?,
        Commands::Test(test) => test::test(test, config).await?,
        Commands::Update(update_args) => update::update(update_args, config).await?,
        Commands::Info(info_data) => info::info(info_data, config).await?,
        Commands::Path(path_data) => path::path(path_data, config).await?,
        Commands::Pin(pin_data) => pin::set_pinned_state(pin_data, config, Pinned).await?,
        Commands::Unpin(pin_data) => pin::set_pinned_state(pin_data, config, Unpinned).await?,
        Commands::Upload(upload_data) => upload::upload(upload_data, config).await?,
        Commands::Check(check_args) => check::check(check_args, config).await?,
        Commands::Add(add_data) => add::add(add_data, config).await?,
        Commands::Config(config_cmd) => config::config(config_cmd, config)?,
        Commands::Doc(doc_args) => doc::doc(doc_args, config).await?,
        Commands::Lint => unimplemented!(),
        Commands::Pack(pack_args) => pack::pack(pack_args, config).await?,
        Commands::Uninstall(uninstall_data) => {
            uninstall::uninstall(uninstall_data, config).await.unwrap()
        }
        Commands::Which(which_args) => which::which(which_args, config)?,
        Commands::Run(run_args) => run::run(run_args, config).await?,
        Commands::GenerateRockspec(data) => generate_rockspec::generate_rockspec(data)?,
        Commands::Shell(data) => shell::shell(data, config).await?,
    }
    Ok(())
}
