use eyre::{eyre, Result};
use lux_lib::package::PackageReq;
use std::{path::PathBuf, str::FromStr};

#[derive(Debug, Clone)]
pub enum PackageOrRockspec {
    Package(PackageReq),
    RockSpec(PathBuf),
}

impl FromStr for PackageOrRockspec {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path = PathBuf::from(s);
        if path.is_file() {
            Ok(Self::RockSpec(path))
        } else {
            let pkg = PackageReq::from_str(s).map_err(|err| {
                eyre!(
                    "No file {0} found and cannot parse package query: {1}",
                    s,
                    err
                )
            })?;
            Ok(Self::Package(pkg))
        }
    }
}
