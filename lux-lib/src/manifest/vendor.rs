use crate::package;
use std::{collections::HashMap, path::Path, str::FromStr};

use walkdir::WalkDir;

use crate::{manifest::Metadata, remote};

/// Construct manifest metadata from a vendor directory.
#[tracing::instrument(level = "trace")]
pub(crate) fn from_dir(vendor_dir: &Path) -> Metadata {
    let mut repository = HashMap::new();
    for (package, package_type) in WalkDir::new(vendor_dir)
        .into_iter()
        .filter_map(|file| file.ok())
        .filter_map(|file| {
            let file = file.path();
            let is_binary = file.extension().is_some_and(|ext| ext == "rock");
            let package_type = if is_binary {
                remote::package::Kind::Binary
            } else {
                remote::package::Kind::Rockspec
            };
            let file_name = if file.is_file() && is_binary {
                file.with_extension("")
            } else {
                file.to_path_buf()
            };
            let file_name = file_name.file_name().unwrap_or_default();
            // NOTE: We silently ignore entries if we can't parse a `Spec` from them.
            let package = package::Spec::from_str(file_name.to_string_lossy().as_ref()).ok()?;
            Some((package, package_type))
        })
    {
        let packages = repository
            .entry(package.name().clone())
            .or_insert_with(HashMap::default);
        packages.insert(package.version().clone(), vec![package_type]);
    }
    Metadata { repository }
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
            vec![remote::package::Kind::Rockspec],
        );
        let foo_dir = vendor_dir.child("foo@1.0.0-1");
        foo_dir.create_dir_all().unwrap();
        foo_pkgs.insert(
            "1.0.0-1".parse().unwrap(),
            vec![remote::package::Kind::Rockspec],
        );
        let bar_dir = vendor_dir.child("bar@2.0.0-2");
        bar_dir.create_dir_all().unwrap();
        let bar_pkgs = expected
            .entry("bar".into())
            .or_insert_with(HashMap::default);
        bar_pkgs.insert(
            "2.0.0-2".parse().unwrap(),
            vec![remote::package::Kind::Rockspec],
        );
        let baz_rock = vendor_dir.child("baz@1.0.0-1.rock");
        baz_rock.touch().unwrap();
        let baz_pkgs = expected
            .entry("baz".into())
            .or_insert_with(HashMap::default);
        baz_pkgs.insert(
            "1.0.0-1".parse().unwrap(),
            vec![remote::package::Kind::Binary],
        );
        let metadata = from_dir(vendor_dir.path());
        assert_eq!(
            metadata,
            Metadata {
                repository: expected
            }
        )
    }
}
