use std::fmt::Display;

use thiserror::Error;

use crate::remote_package_db::RemotePackageDB;

use super::{version::PackageVersion, PackageName, PackageReq, PackageSpec, PackageVersionReq};

#[derive(Error, Debug)]
#[error("rock {0} not found")]
pub struct RockNotFound(PackageName);

#[derive(Error, Debug)]
#[error("rock {name} has no version that satisfies constraint {constraint}")]
pub struct RockConstraintUnsatisfied {
    name: PackageName,
    constraint: PackageVersionReq,
}

impl PackageSpec {
    /// Tries to find a newer version of a given rock.
    /// Returns the latest version if found.
    pub fn has_update(
        &self,
        package_db: &RemotePackageDB,
    ) -> Result<Option<PackageVersion>, RockNotFound> {
        let latest_version = package_db
            .latest_version(&self.name)
            .ok_or_else(|| RockNotFound(self.name.clone()))?;

        if self.version < *latest_version {
            Ok(Some(latest_version.to_owned()))
        } else {
            Ok(None)
        }
    }

    /// Tries to find a newer version of a rock given a constraint.
    /// Returns the latest version if found.
    pub fn has_update_with(
        &self,
        constraint: &PackageReq,
        package_db: &RemotePackageDB,
    ) -> Result<Option<PackageVersion>, RockConstraintUnsatisfied> {
        let latest_version =
            package_db
                .latest_match(constraint, None)
                .ok_or_else(|| RockConstraintUnsatisfied {
                    name: self.name.clone(),
                    constraint: constraint.version_req.clone(),
                })?;

        if self.version < latest_version.version {
            Ok(Some(latest_version.version))
        } else {
            Ok(None)
        }
    }
}

impl Display for PackageSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{} {}", self.name, self.version).as_str())
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use url::Url;

    use crate::{
        manifest::{Manifest, ManifestMetadata},
        package::PackageSpec,
    };

    #[test]
    fn rock_has_update() {
        let test_manifest_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/manifest-5.1");
        let content = String::from_utf8(std::fs::read(&test_manifest_path).unwrap()).unwrap();
        let metadata = ManifestMetadata::new(&content).unwrap();
        let package_db = Manifest::new(Url::parse("https://example.com").unwrap(), metadata).into();

        let test_package =
            PackageSpec::parse("lua-cjson".to_string(), "2.0.0".to_string()).unwrap();

        assert_eq!(
            test_package.has_update(&package_db).unwrap(),
            Some("2.1.0-1".parse().unwrap())
        );
    }
}
