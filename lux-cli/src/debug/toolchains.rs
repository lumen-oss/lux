use crate::args::OutputFormat;
use clap::Args;
use eyre::Result;
use lux_lib::toolchains::{Tool, ToolchainReport};

#[derive(Args)]
pub struct Toolchains {
    /// The output format.
    #[arg(long, default_value = "text", value_enum, ignore_case = true)]
    output_format: OutputFormat,
}

impl Toolchains {
    pub fn output_format(&self) -> OutputFormat {
        self.output_format.clone()
    }
}

pub fn check_toolchains(args: Toolchains) -> Result<()> {
    let report = ToolchainReport::generate();

    match args.output_format() {
        OutputFormat::Text => print_human_readable(&report),
        OutputFormat::Json => print_json(&report)?,
    }

    Ok(())
}

fn print_human_readable(report: &ToolchainReport) {
    println!("Toolchains Report");
    println!("=======================\n");

    print_toolchains(report.c_compiler());
    print_toolchains(report.make());
    print_toolchains(report.cmake());
    print_toolchains(report.cargo());
    print_toolchains(report.pkg_config());

    println!("\nSummary:");
    let total = 5;

    let found = [
        report.c_compiler(),
        report.make(),
        report.cmake(),
        report.cargo(),
        report.pkg_config(),
    ]
    .iter()
    .filter(|d| d.is_found())
    .count();

    println!("  Found: {}/{}", found, total);
    println!("  Missing: {}/{}", total - found, total);
}

fn print_toolchains(dep: &Tool) {
    match dep.info() {
        Some(info) => {
            print!("✓ {}: Found ({})", dep.name(), info.path().display());
            if let Some(version) = info.version() {
                print!(" - {}", version);
            }
            println!();
        }
        None => println!("✗ {}: Not found", dep.name()),
    }
}

fn print_json(report: &ToolchainReport) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    println!("{}", json);
    Ok(())
}
