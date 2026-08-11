use std::io::IsTerminal;

use std::time::Duration;

use clap::Parser;
use lux_cli::{
    add, build, check, config,
    debug::{self, Debug},
    dist::{self, Dist},
    doc, download, exec, fetch, format, generate_rockspec, info, install, install_lua,
    install_rockspec, lint, list, outdated, pack, path, pin, progress, project, purge, remove, run,
    run_lua, search, shell, sync, test, uninstall, unpack, update,
    upload::{self},
    util, vendor, which, Cli, Commands,
};
use lux_lib::{
    config::tree::RockLayoutConfig,
    lockfile::PinnedState::{Pinned, Unpinned},
    lua_version::LuaVersion,
};

use miette::{IntoDiagnostic, MietteHandlerOpts, Result};
use tracing::{span::Id, Subscriber};
use tracing_indicatif::span_ext::IndicatifSpanExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
    Layer,
};

use lux_cli::utils::error::clap_to_miette;

const DEFAULT_USER_AGENT: &str = concat!("lux/", env!("CARGO_PKG_VERSION"));
fn main() -> Result<()> {
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

    let lua_version = cli
        .lua_version
        .or({
            if cli.nvim {
                Some(LuaVersion::Lua51)
            } else {
                None
            }
        })
        .or_else(|| cli.command.lua_version());

    let mut config_builder = cli
        .command
        .config()?
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
        .user_tree(cli.tree.clone())
        .workspace_tree(cli.tree)
        .variables(
            cli.variables
                .map(|variables| variables.into_iter().collect()),
        )
        .verbose(Some(cli.verbose))
        .no_progress(Some(cli.no_progress))
        .no_prompt(Some(
            cli.no_prompt.unwrap_or(!std::io::stderr().is_terminal()),
        ))
        .user_agent(Some(cli.user_agent.unwrap_or(DEFAULT_USER_AGENT.into())))
        .no_tfa(Some(cli.no_tfa));

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

    if config.no_progress() || !std::io::stderr().is_terminal() {
        tracing_subscriber::registry().with(fmt_layer).init();
    } else {
        let indicatif_layer = progress::IndicatifLayer::new()
            .with_progress_style(
                indicatif::ProgressStyle::with_template(
                    "{spinner} {span_child_prefix}{span_name} {{{span_fields}}}",
                )
                .into_diagnostic()?
                .tick_chars("🌑🌒🌓🌔🌕🌖🌗🌘"),
            )
            .with_tick_settings(tracing_indicatif::TickSettings {
                term_draw_hz: 20,
                default_tick_interval: Some(Duration::from_millis(165)),
                footer_tick_interval: Some(Duration::from_millis(165)),
                ..Default::default()
            });
        let fmt_layer = tracing_subscriber::fmt::layer::<tracing_subscriber::Registry>()
            .with_target(false)
            .with_writer(indicatif_layer.get_stderr_writer())
            .with_filter(fmt_filter);
        let span_fmt_filter_level = if config.verbose() { "trace" } else { "info" };
        let indicatif_layer = indicatif_layer.with_filter(
            tracing_subscriber::filter::EnvFilter::new(span_fmt_filter_level),
        );
        tracing_subscriber::registry()
            .with(fmt_layer)
            .with(ProgressStyleTemplateLayer {})
            .with(indicatif_layer)
            .init();
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .into_diagnostic()?;

    match cli.command {
        Commands::Check(check_args) => rt.block_on(check::check(check_args, config))?,
        Commands::Search(search_data) => rt.block_on(search::search(search_data, config))?,
        Commands::Download(download_data) => {
            rt.block_on(download::download(download_data, config))?
        }
        Commands::Debug(debug) => match debug {
            Debug::FetchRemote(unpack_data) => {
                rt.block_on(fetch::fetch_remote(unpack_data, config))?
            }
            Debug::Unpack(unpack_data) => rt.block_on(unpack::unpack(unpack_data, config))?,
            Debug::UnpackRemote(unpack_data) => {
                rt.block_on(unpack::unpack_remote(unpack_data, config))?
            }
            Debug::Project(debug_project) => project::debug_project(debug_project)?,
            Debug::Toolchains(tool_args) => debug::toolchains::check_toolchains(tool_args)?,
        },
        Commands::Dist(dist_data) => match dist_data {
            Dist::FlatArchive(archive) => rt.block_on(dist::dist_archive(archive, config))?,
            Dist::Bin(bin) => rt.block_on(dist::bin(bin, config))?,
        },
        Commands::New(project_data) => {
            rt.block_on(project::write_project_rockspec(project_data, config))?
        }
        Commands::Build(build_data) => {
            rt.block_on(build::build(build_data, config))?;
        }
        Commands::List(list_data) => list::list_installed(list_data, config)?,
        Commands::Lua(run_lua) => rt.block_on(run_lua::run_lua(run_lua, config))?,
        Commands::Install(install_data) => rt.block_on(install::install(install_data, config))?,
        Commands::InstallRockspec(install_data) => {
            rt.block_on(install_rockspec::install_rockspec(install_data, config))?
        }
        Commands::Outdated(outdated) => rt.block_on(outdated::outdated(outdated, config))?,
        Commands::InstallLua => rt.block_on(install_lua::install_lua(config))?,
        Commands::Fmt(fmt_args) => format::format(fmt_args, config)?,
        Commands::Purge => rt.block_on(purge::purge(config))?,
        Commands::Remove(remove_args) => rt.block_on(remove::remove(remove_args, config))?,
        Commands::Exec(run_args) => rt.block_on(exec::exec(run_args, config))?,
        Commands::Test(test) => rt.block_on(test::test(test, config))?,
        Commands::Update(update_args) => rt.block_on(update::update(update_args, config))?,
        Commands::Info(info_data) => rt.block_on(info::info(info_data, config))?,
        Commands::Lint(lint_args) => rt.block_on(lint::lint(lint_args, config))?,
        Commands::Path(path_data) => rt.block_on(path::path(path_data, config))?,
        Commands::Pin(pin_data) => rt.block_on(pin::set_pinned_state(pin_data, config, Pinned))?,
        Commands::Unpin(pin_data) => {
            rt.block_on(pin::set_pinned_state(pin_data, config, Unpinned))?
        }
        Commands::Upload(upload_data) => rt.block_on(upload::upload(upload_data, config))?,
        Commands::Add(add_data) => rt.block_on(add::add(add_data, config))?,
        Commands::Config(config_cmd) => config::config(config_cmd, config)?,
        Commands::Doc(doc_args) => rt.block_on(doc::doc(doc_args, config))?,
        Commands::Pack(pack_args) => rt.block_on(pack::pack(pack_args, config))?,
        Commands::Uninstall(uninstall_data) => {
            rt.block_on(uninstall::uninstall(uninstall_data, config))?
        }
        Commands::Util(util) => rt.block_on(util::util(util, config))?,
        Commands::Vendor(vendor_args) => rt.block_on(vendor::vendor(vendor_args, config))?,
        Commands::Which(which_args) => which::which(which_args, config)?,
        Commands::Run(run_args) => rt.block_on(run::run(run_args, config))?,
        Commands::GenerateRockspec(data) => {
            rt.block_on(generate_rockspec::generate_rockspec(data))?
        }
        Commands::Shell(data) => rt.block_on(shell::shell(data, config))?,
        Commands::Sync(sync_args) => rt.block_on(sync::sync(sync_args, config))?,
    }
    Ok(())
}

/// Checks if the current span has any fields, and if it doesn't, sets the template to exclude them
struct ProgressStyleTemplateLayer {}

impl<S> Layer<S> for ProgressStyleTemplateLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_enter(&self, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if span.fields().is_empty() {
                if let Ok(style) =
                    &indicatif::ProgressStyle::with_template("{span_child_prefix}{span_name}")
                {
                    tracing::Span::current().pb_set_style(style);
                }
            }
        }
    }
}
