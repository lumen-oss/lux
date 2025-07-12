use std::{path::PathBuf, str::FromStr};

use ::serde::Deserialize;
use git2::Repository;
use serde::Serialize;
use thiserror::Error;

use crate::{
    lua_rockspec::{RockSourceInternal, SourceUrl, SourceUrlError},
    package::{PackageName, PackageSpec, PackageVersion, PackageVersionParseError},
    variables::{self, Environment, HasVariables, VariableSubstitutionError},
};

use super::ProjectRoot;

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone, Default)]
/// Template for generating a remote rockspec source
///
/// Variables that can be substituted in each of the fields:
/// - `$(PACKAGE)`: Package name
/// - `$(VERSION)`: Package version
/// - `$(REF)`: Git tag or revision (prioritising tags if present)
///
/// Fields can also be substituted with environment variables.
pub(crate) struct RockSourceTemplate {
    /// URL template for `SemVer` releases
    url: Option<String>,

    /// URL template for `DevVer` releases
    dev: Option<String>,

    /// File name of the source archive.
    /// Can be omitted if it can be inferred from the generated URL.
    file: Option<PathBuf>,

    /// Name of the directory created when the source archive is unpacked.
    /// Can be omitted if it can be inferred from the `file` field.
    dir: Option<PathBuf>,

    /// The tag or revision to be checked out if the source URL is a git source.
    /// If unset, Lux will try to auto-detect it.
    tag: Option<String>,
}

#[derive(Debug, Error)]
pub enum GenerateSourceError {
    #[error(
        "unsupported version {0}.\nCan only generate source for SemVer versions, 'dev' or 'scm'."
    )]
    StringVer(String),
    #[error("need a `source.url` (release URL) in lux.toml for SemVer versions.")]
    MissingReleaseUrl(String),
    #[error("need a `source.dev` (dev/scm URL) in lux.toml for dev versions.")]
    MissingDevUrl(String),
    #[error("error substituting project source variables:\n{0}")]
    VariableSubstitution(#[from] VariableSubstitutionError),
    #[error("error parsing source URL from template:\n{0}")]
    SourceUrl(#[from] SourceUrlError),
    #[error("error generating git source URL:\n{0}")]
    Git(#[from] git2::Error),
    #[error("refusing to generate nondeterministic rockspec with git source.\nSupply a `source.tag` parameter.")]
    NonDeterministicGitSource,
}

/// Helper for substituting git variables from a git project
struct GitProject<'a>(&'a ProjectRoot);

impl HasVariables for GitProject<'_> {
    fn get_variable(&self, input: &str) -> Option<String> {
        match input {
            "REF" => Repository::open(self.0)
                .ok()
                .and_then(|repo| current_tag_or_revision(&repo).ok()),
            _ => None,
        }
    }
}

impl RockSourceTemplate {
    pub(crate) fn try_generate(
        &self,
        project_root: &ProjectRoot,
        package: &PackageName,
        version: &PackageVersion,
    ) -> Result<RockSourceInternal, GenerateSourceError> {
        let package_spec = PackageSpec::new(package.clone(), version.clone());
        let url_template_str = match version {
            PackageVersion::SemVer(ver) => self
                .url
                .as_ref()
                .ok_or(GenerateSourceError::MissingReleaseUrl(ver.to_string())),
            PackageVersion::DevVer(ver) => self
                .dev
                .as_ref()
                .ok_or(GenerateSourceError::MissingDevUrl(ver.to_string())),
            PackageVersion::StringVer(ver) => Err(GenerateSourceError::StringVer(ver.to_string())),
        }?;
        let url_str = variables::substitute(
            &[&package_spec, &Environment {}, &GitProject(project_root)],
            url_template_str,
        )?;
        match SourceUrl::from_str(&url_str)? {
            SourceUrl::File(_) | SourceUrl::Url(_) => Ok(RockSourceInternal {
                url: Some(url_str.to_string()),
                file: self.file.clone(),
                dir: self.dir.clone(),
                branch: None,
                tag: None,
            }),
            SourceUrl::Git(_) if self.tag.is_none() => {
                if let Ok(repo) = Repository::open(project_root) {
                    let tag_or_rev = current_tag_or_revision(&repo)?;
                    Ok(RockSourceInternal {
                        url: Some(url_str.to_string()),
                        tag: Some(tag_or_rev),
                        file: self.file.clone(),
                        dir: self.dir.clone(),
                        branch: None,
                    })
                } else {
                    Err(GenerateSourceError::NonDeterministicGitSource)
                }
            }
            SourceUrl::Git(_) => Ok(RockSourceInternal {
                url: Some(url_str.to_string()),
                file: self.file.clone(),
                dir: self.dir.clone(),
                tag: self.tag.clone(),
                branch: None,
            }),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize, Serialize, Clone, Default)]
pub(crate) struct PackageVersionTemplate(Option<PackageVersion>);

#[derive(Debug, Error)]
pub enum GenerateVersionError {
    #[error("error generating version from git repository metadata:\n{0}")]
    Git(#[from] git2::Error),
    #[error("error parsing version from git ref:\n{0}")]
    PackageVersionParse(#[from] PackageVersionParseError),
}

impl PackageVersionTemplate {
    pub(crate) fn try_generate(
        &self,
        project_root: &ProjectRoot,
    ) -> Result<PackageVersion, GenerateVersionError> {
        if let Some(version) = &self.0 {
            Ok(version.clone())
        } else {
            let repo = Repository::open(project_root)?;
            if let Some(version) = version_from_semver_tag(&repo)? {
                Ok(version)
            } else {
                Ok(PackageVersion::default_dev_version())
            }
        }
    }
}

/// Searches the current HEAD for SemVer tags and returns the first one found.
fn version_from_semver_tag(repo: &Repository) -> Result<Option<PackageVersion>, git2::Error> {
    let head = repo.head()?;
    let current_rev = head
        .target()
        .ok_or_else(|| git2::Error::from_str("No HEAD target"))?;
    let mut result = None;
    repo.tag_foreach(|oid, _| {
        if let Ok(obj) = repo.find_object(oid, None) {
            let tag = obj.into_tag().expect("not a tag");
            if tag.target_id() == current_rev {
                if let Some(tag_name) = tag.name() {
                    if let Ok(version @ PackageVersion::SemVer(_)) =
                        PackageVersion::parse(tag_name.trim_start_matches("v"))
                    {
                        result = Some(version);
                        return false; // stop iteration
                    }
                }
            }
        }
        true // continue iteration
    })?;
    Ok(result)
}

/// Searches the current HEAD for a tag, and if found, returns it.
/// Prioritises SemVer tags.
/// Returns the HEAD's commit SHA if no tag is found.
fn current_tag_or_revision(repo: &Repository) -> Result<String, git2::Error> {
    let head = repo.head()?;
    let current_rev = head
        .target()
        .ok_or_else(|| git2::Error::from_str("No HEAD target"))?;
    let mut semver_tag = None;
    let mut fallback_tag = None;
    repo.tag_foreach(|oid, _| {
        if let Ok(obj) = repo.find_object(oid, None) {
            let tag = obj.into_tag().expect("not a tag");
            if tag.target_id() == current_rev {
                if let Some(tag_name) = tag.name() {
                    if PackageVersion::parse(tag_name.trim_start_matches("v"))
                        .is_ok_and(|version| version.is_semver())
                    {
                        semver_tag = Some(tag_name.to_string());
                        return false; // stop iteration
                    }
                    fallback_tag = Some(tag_name.to_string());
                }
            }
        }
        true // continue iteration
    })?;
    Ok(semver_tag
        .or(fallback_tag)
        .unwrap_or(current_rev.to_string()))
}
