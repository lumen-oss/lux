use clap::ValueEnum;
use lux_lib::package::PackageReq;
use miette::{miette, Result};
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Clone)]
pub enum PackageOrRockspec {
    Package(PackageReq),
    RockSpec(PathBuf),
}

#[derive(Debug, Clone, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Text,
}

impl FromStr for PackageOrRockspec {
    type Err = miette::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path = PathBuf::from(s);
        if path.is_file() {
            Ok(Self::RockSpec(path))
        } else {
            let pkg = PackageReq::from_str(s).map_err(|err| {
                miette!(
                    help = format!("if '{s}' is a path to a file, ensure it exists"),
                    "No file '{s}' found and cannot parse package query: {err}",
                )
            })?;
            Ok(Self::Package(pkg))
        }
    }
}
