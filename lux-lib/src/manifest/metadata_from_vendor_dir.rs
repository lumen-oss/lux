use std::{collections::HashMap, path::Path, str::FromStr};

use walkdir::WalkDir;

use crate::{
    manifest::ManifestMetadata,
    package::{PackageSpec, RemotePackageType},
};

/// Construct manifest metadata from a vendor directory.
pub(crate) fn manifest_from_vendor_dir(vendor_dir: &Path) -> ManifestMetadata {
    let mut repository = HashMap::new();
    for (package, package_type) in WalkDir::new(vendor_dir)
        .into_iter()
        .filter_map(|file| file.ok())
        .filter_map(|file| {
            let mut file = file.path().to_path_buf();
            let package_type = match file.extension() {
                Some(ext) if ext == "rock" => RemotePackageType::Binary,
                _ => RemotePackageType::Rockspec,
            };
            if file.is_file() {
                // So we can find .rock archives
                file.set_extension("");
            }
            let file_name = file.file_name().unwrap_or_default();
            // NOTE: We silently ignore entries if we can't parse a `PackageSpec` from them.
            let package = PackageSpec::from_str(file_name.to_string_lossy().as_ref()).ok()?;
            Some((package, package_type))
        })
    {
        let packages = repository
            .entry(package.name().clone())
            .or_insert_with(HashMap::default);
        packages.insert(package.version().clone(), vec![package_type]);
    }
    ManifestMetadata { repository }
}

#[cfg(test)]
mod tests {
    use assert_fs::{
        prelude::{FileTouch, PathChild, PathCreateDir},
        TempDir,
    };

    use super::*;

    #[tokio::test]
    async fn test_find_vendored_package() {
        let vendor_dir = TempDir::new().unwrap();
        let mut expected = HashMap::new();
        let invalid_foo_dir = vendor_dir.child("foo-0.1.0-1");
        invalid_foo_dir.create_dir_all().unwrap();
        let outdated_foo_dir = vendor_dir.child("foo@0.1.0-1");
        outdated_foo_dir.create_dir_all().unwrap();
        let foo_pkgs = expected
            .entry("foo".into())
            .or_insert_with(HashMap::default);
        foo_pkgs.insert(
            "0.1.0-1".parse().unwrap(),
            vec![RemotePackageType::Rockspec],
        );
        let foo_dir = vendor_dir.child("foo@1.0.0-1");
        foo_dir.create_dir_all().unwrap();
        foo_pkgs.insert(
            "1.0.0-1".parse().unwrap(),
            vec![RemotePackageType::Rockspec],
        );
        let bar_dir = vendor_dir.child("bar@2.0.0-2");
        bar_dir.create_dir_all().unwrap();
        let bar_pkgs = expected
            .entry("bar".into())
            .or_insert_with(HashMap::default);
        bar_pkgs.insert(
            "2.0.0-2".parse().unwrap(),
            vec![RemotePackageType::Rockspec],
        );
        let baz_rock = vendor_dir.child("baz@1.0.0-1.rock");
        baz_rock.touch().unwrap();
        let baz_pkgs = expected
            .entry("baz".into())
            .or_insert_with(HashMap::default);
        baz_pkgs.insert("1.0.0-1".parse().unwrap(), vec![RemotePackageType::Binary]);
        let metadata = manifest_from_vendor_dir(vendor_dir.path());
        assert_eq!(
            metadata,
            ManifestMetadata {
                repository: expected
            }
        )
    }
}
