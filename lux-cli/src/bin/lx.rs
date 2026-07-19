use std::time::Duration;

use clap::Parser;
use lux_cli::{
    add, build, check, completion, config,
    debug::{self, Debug},
    dist::{self, Dist},
    doc, download, exec, fetch, format, generate_rockspec, info, install, install_lua,
    install_rockspec, lint, list, outdated, pack, path, pin, progress, project, purge, remove, run,
    run_lua, search, shell, sync, test, uninstall, unpack, update,
    upload::{self},
    vendor, which, Cli, Commands,
};
use lux_lib::{
    config::{tree::RockLayoutConfig, ConfigBuilder},
    lockfile::PinnedState::{Pinned, Unpinned},
    lua_version::LuaVersion,
};
use miette::{IntoDiagnostic, MietteHandlerOpts, Result};
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

use lux_cli::utils::error::clap_to_miette;

const DEFAULT_USER_AGENT: &str = concat!("lux/", env!("CARGO_PKG_VERSION"));
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    miette::set_hook(Box::new(|_| {
        Box::new(
            MietteHandlerOpts::new()
                .terminal_links(true)
                .unicode(true)
                .context_lines(3)
                .tab_width(4)
                .break_words(true)
                .with_cause_chain()
                .build(),
        )
    }))?;
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if !err.use_stderr() {
                let _ = err.print();
                return Ok(());
            }

            match clap_to_miette(err) {
                Ok(report) => return Err(report),
                Err(text) => {
                    print!("{text}");
                    return Err(miette::miette!("not enough arguments supplied"));
                }
            }
        }
    };

    let lua_version = cli.lua_version.or({
        if cli.nvim {
            Some(LuaVersion::Lua51)
        } else {
            None
        }
    });

    let mut config_builder = ConfigBuilder::new()?
        .dev(Some(cli.dev))
        .extra_servers(cli.extra_servers)
        .generate_luarc(Some(!cli.no_luarc))
        .wrap_bin_scripts(Some(!cli.no_wrap_bin))
        .lua_dir(cli.lua_dir)
        .lua_version(lua_version)
        .namespace(cli.namespace)
        .cache_dir(cli.cache_dir)
        .data_dir(cli.data_dir)
        .vendor_dir(cli.vendor_dir)
        .server(cli.server)
        .timeout(
            cli.timeout
                .map(|duration| Duration::from_secs(duration as u64)),
        )
        .max_jobs(cli.max_jobs)
        .user_tree(cli.tree)
        .variables(
            cli.variables
                .map(|variables| variables.into_iter().collect()),
        )
        .verbose(Some(cli.verbose))
        .no_progress(Some(cli.no_progress))
        .no_prompt(Some(cli.no_prompt))
        .user_agent(Some(cli.user_agent.unwrap_or(DEFAULT_USER_AGENT.into())));

    if cli.nvim {
        config_builder = config_builder.entrypoint_layout(RockLayoutConfig::new_nvim_layout());
    }

    let config = config_builder.build()?;

    if config.verbose() {
        std::env::set_var("CC_ENABLE_DEBUG_OUTPUT", "1");
    }

    let fmt_filter = if config.verbose() {
        tracing_subscriber::filter::EnvFilter::new("debug")
    } else {
        tracing_subscriber::filter::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::filter::EnvFilter::new("warn"))
    };

    let fmt_layer = tracing_subscriber::fmt::layer::<tracing_subscriber::Registry>()
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_filter(fmt_filter.clone());

    if config.no_progress() {
        tracing_subscriber::registry().with(fmt_layer).init();
    } else {
        let indicatif_layer = progress::IndicatifLayer::new().with_progress_style(
            indicatif::ProgressStyle::with_template(
                "{spinner} {span_child_prefix}{span_name}... {span_fields}",
            )
            .into_diagnostic()?
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        let fmt_layer = tracing_subscriber::fmt::layer::<tracing_subscriber::Registry>()
            .with_target(false)
            .with_writer(indicatif_layer.get_stderr_writer())
            .with_filter(fmt_filter);
        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(indicatif_layer)
            .init();
    }

    match cli.command {
        Commands::Check(check_args) => check::check(check_args, config).await?,
        Commands::Completion(completion_args) => completion::completion(completion_args).await?,
        Commands::Search(search_data) => search::search(search_data, config).await?,
        Commands::Download(download_data) => download::download(download_data, config).await?,
        Commands::Debug(debug) => match debug {
            Debug::FetchRemote(unpack_data) => fetch::fetch_remote(unpack_data, config).await?,
            Debug::Unpack(unpack_data) => unpack::unpack(unpack_data, config).await?,
            Debug::UnpackRemote(unpack_data) => unpack::unpack_remote(unpack_data, config).await?,
            Debug::Project(debug_project) => project::debug_project(debug_project)?,
            Debug::Toolchains(tool_args) => debug::toolchains::check_toolchains(tool_args)?,
        },
        Commands::Dist(dist_data) => match dist_data {
            Dist::FlatArchive(archive) => dist::dist_archive(archive, config).await?,
            Dist::Bin(bin) => dist::bin(bin, config).await?,
        },
        Commands::New(project_data) => {
            project::write_project_rockspec(project_data, config).await?
        }
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
        Commands::Fmt(fmt_args) => format::format(fmt_args, config)?,
        Commands::Purge => purge::purge(config).await?,
        Commands::Remove(remove_args) => remove::remove(remove_args, config).await?,
        Commands::Exec(run_args) => exec::exec(run_args, config).await?,
        Commands::Test(test) => test::test(test, config).await?,
        Commands::Update(update_args) => update::update(update_args, config).await?,
        Commands::Info(info_data) => info::info(info_data, config).await?,
        Commands::Lint(lint_args) => lint::lint(lint_args, config).await?,
        Commands::Path(path_data) => path::path(path_data, config).await?,
        Commands::Pin(pin_data) => pin::set_pinned_state(pin_data, config, Pinned).await?,
        Commands::Unpin(pin_data) => pin::set_pinned_state(pin_data, config, Unpinned).await?,
        Commands::Upload(upload_data) => upload::upload(upload_data, config).await?,
        Commands::Add(add_data) => add::add(add_data, config).await?,
        Commands::Config(config_cmd) => config::config(config_cmd, config)?,
        Commands::Doc(doc_args) => doc::doc(doc_args, config).await?,
        Commands::Pack(pack_args) => pack::pack(pack_args, config).await?,
        Commands::Uninstall(uninstall_data) => uninstall::uninstall(uninstall_data, config).await?,
        Commands::Vendor(vendor_args) => vendor::vendor(vendor_args, config).await?,
        Commands::Which(which_args) => which::which(which_args, config)?,
        Commands::Run(run_args) => run::run(run_args, config).await?,
        Commands::GenerateRockspec(data) => generate_rockspec::generate_rockspec(data).await?,
        Commands::Shell(data) => shell::shell(data, config).await?,
        Commands::Sync(sync_args) => sync::sync(sync_args, config).await?,
    }
    Ok(())
}
