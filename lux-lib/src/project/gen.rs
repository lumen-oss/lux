use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use ::serde::Deserialize;
use git2::Repository;
use serde::Serialize;
use thiserror::Error;

use crate::{
    lua_rockspec::{RockSourceInternal, SourceUrl, SourceUrlError},
    package::{PackageName, PackageSpec, PackageVersion, PackageVersionParseError, SpecRev},
    variables::{self, Environment, GetVariableError, HasVariables, VariableSubstitutionError},
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
    fn get_variable(&self, input: &str) -> Result<Option<String>, GetVariableError> {
        Ok(match input {
            "REF" => {
                let repo = find_git_repo(self.0).map_err(GetVariableError::new)?;
                Some(current_tag_or_revision(&repo).map_err(GetVariableError::new)?)
            }
            _ => None,
        })
    }
}

fn find_git_repo(path: impl AsRef<Path>) -> Result<Repository, git2::Error> {
    let mut path: PathBuf = path.as_ref().to_path_buf();
    loop {
        match Repository::open(&path) {
            Ok(repo) => return Ok(repo),
            Err(err) => {
                if !path.pop() {
                    return Err(err);
                }
            }
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
        let dir = match self.dir.as_ref() {
            Some(dir) => Some(
                variables::substitute(
                    &[&package_spec, &Environment {}, &GitProject(project_root)],
                    &dir.to_string_lossy(),
                )?
                .into(),
            ),
            None => None,
        };
        let file = match self.file.as_ref() {
            Some(file) => Some(
                variables::substitute(
                    &[&package_spec, &Environment {}, &GitProject(project_root)],
                    &file.to_string_lossy(),
                )?
                .into(),
            ),
            None => None,
        };
        let tag = match self.tag.as_ref() {
            Some(tag) => Some(variables::substitute(
                &[&package_spec, &Environment {}, &GitProject(project_root)],
                tag,
            )?),
            None => None,
        };
        match SourceUrl::from_str(&url_str)? {
            SourceUrl::File(_) | SourceUrl::Url(_) => Ok(RockSourceInternal {
                url: Some(url_str.to_string()),
                file,
                dir,
                branch: None,
                tag,
            }),
            SourceUrl::Git(_) if self.tag.is_none() => {
                if let Ok(repo) = Repository::open(project_root) {
                    let tag_or_rev = current_tag_or_revision(&repo)?;
                    Ok(RockSourceInternal {
                        url: Some(url_str.to_string()),
                        tag: Some(tag_or_rev),
                        file,
                        dir,
                        branch: None,
                    })
                } else {
                    Err(GenerateSourceError::NonDeterministicGitSource)
                }
            }
            SourceUrl::Git(_) => Ok(RockSourceInternal {
                url: Some(url_str.to_string()),
                file,
                dir,
                tag,
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
        specrev: Option<SpecRev>,
    ) -> Result<PackageVersion, GenerateVersionError> {
        let specrev = specrev.unwrap_or_default();
        if let Some(version) = &self.0 {
            Ok(version.clone())
        } else {
            let repo = find_git_repo(project_root)?;
            if let Some(version) = version_from_semver_tag(&repo, &specrev)? {
                Ok(version)
            } else {
                Ok(PackageVersion::default_dev_version_with_specrev(specrev))
            }
        }
    }
}

/// Searches the current HEAD for SemVer tags and returns the first one found.
fn version_from_semver_tag(
    repo: &Repository,
    specrev: &SpecRev,
) -> Result<Option<PackageVersion>, git2::Error> {
    let head = repo.head()?;
    let current_rev = head
        .target()
        .ok_or_else(|| git2::Error::from_str("No HEAD target"))?;
    let mut result = None;
    repo.tag_foreach(|oid, name| {
        let name = std::str::from_utf8(name).ok().map(|x| x.to_string());
        if let Some(tag_name) = name {
            if oid == current_rev {
                let version_str =
                    format!("{}-{specrev}", tag_name.trim_start_matches("refs/tags/v"));
                if let Ok(version @ PackageVersion::SemVer(_)) = PackageVersion::parse(&version_str)
                {
                    result = Some(version);
                    return false; // stop iteration
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
    repo.tag_foreach(|oid, name| {
        let name = std::str::from_utf8(name).ok().map(|x| x.to_string());
        if oid == current_rev {
            if let Some(tag_name) = name {
                if PackageVersion::parse(tag_name.trim_start_matches("refs/tags/v"))
                    .is_ok_and(|version| version.is_semver())
                {
                    semver_tag = Some(tag_name.to_string());
                    return false; // stop iteration
                }
                fallback_tag = Some(tag_name.to_string());
            }
        }
        true // continue iteration
    })?;
    Ok(semver_tag
        .or(fallback_tag)
        .unwrap_or(current_rev.to_string()))
}
