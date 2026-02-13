use crate::debug::{Dependencies, OutputFormat};
use eyre::Result;
use lux_lib::dependencies::DependencyReport;

pub fn check_dependencies(args: Dependencies) -> Result<()> {
    let report = DependencyReport::generate();

    match args.format {
        OutputFormat::Human => print_human_readable(&report),
        OutputFormat::Json => print_json(&report)?,
    }

    Ok(())
}

fn print_human_readable(report: &DependencyReport) {
    println!("Dependency Check Report");
    println!("=======================\n");

    print_dependency(&report.c_compiler);
    print_dependency(&report.make);
    print_dependency(&report.cmake);
    print_dependency(&report.cargo);
    print_dependency(&report.pkg_config);

    println!("\nSummary:");
    let total = 5;
    let found = [
        &report.c_compiler,
        &report.make,
        &report.cmake,
        &report.cargo,
        &report.pkg_config,
    ]
    .iter()
    .filter(|d| d.found)
    .count();

    println!("  Found: {}/{}", found, total);
    println!("  Missing: {}/{}", total - found, total);
}

fn print_dependency(dep: &lux_lib::dependencies::DependencyStatus) {
    let status = if dep.found { "✓" } else { "✗" };
    let status_text = if dep.found { "Found" } else { "Not found" };

    print!("{} {}: {}", status, dep.name, status_text);
    if let Some(path) = &dep.path {
        print!(" ({})", path.display());
    }
    println!();
}

fn print_json(report: &DependencyReport) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    println!("{}", json);
    Ok(())
}
