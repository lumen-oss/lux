use crate::{args::OutputFormat, debug::Toolchains};
use eyre::Result;
use lux_lib::dependencies::{Tool, ToolchainReport};

pub fn check_dependencies(args: Toolchains) -> Result<()> {
    let report = ToolchainReport::generate();

    match args.format {
        OutputFormat::Text => print_human_readable(&report),
        OutputFormat::Json => print_json(&report)?,
    }

    Ok(())
}

fn print_human_readable(report: &ToolchainReport) {
    println!("Toolchains Report");
    println!("=======================\n");

    print_dependency(report.c_compiler());
    print_dependency(report.make());
    print_dependency(report.cmake());
    print_dependency(report.cargo());
    print_dependency(report.pkg_config());

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

fn print_dependency(dep: &Tool) {
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
