use miette::Diagnostic;
use thiserror::Error;

use super::PackageDB;
use crate::manifest;
use crate::operations::install;
use crate::package;
use crate::pipeline::discover::{self, FoundPackage};
use crate::remote;

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("no rock that matches '{0}' found in the lockfile or in the manifests")]
    RockNotFound(package::Req),
    #[error("error when pulling manifest")]
    #[diagnostic(forward(0))]
    Manifest(#[from] manifest::Error),
}

impl PackageDB {
    /// Find a remote package that matches the requirement, returning the latest match.
    #[tracing::instrument(
          name = "Searching",
          level = "info",
          skip_all,
          fields(package = package_req.to_string()),
    )]
    pub(crate) async fn find(
        &self,
        package_req: &package::Req,
        filter: Option<remote::package::kind::FilterSpec>,
    ) -> Result<remote::Package, Error> {
        for provider in &self.providers {
            if let Some(package) = provider.find(package_req, filter.clone()).await? {
                return Ok(package);
            }
        }

        Err(Error::RockNotFound(package_req.clone()))
    }

    /// Resolves a package install spec to a package that is already available.
    pub(crate) async fn resolve(
        &self,
        spec: &install::Spec,
    ) -> Result<Option<FoundPackage>, discover::Error> {
        for provider in &self.providers {
            // Providers that can serve a package without downloading it (local packages)
            // resolve early.
            if let Some(found) = provider.resolve(spec).await? {
                return Ok(Some(found));
            }

            // Otherwise break on the first provider that has the package. The `break` prevents a
            // further invocation of `provider.resolve` on a provider we don't want.
            if provider.find(&spec.package, None).await?.is_some() {
                break;
            }
        }
        Ok(None)
    }

    /// Search for all packages that match the requirement.
    pub async fn search(
        &self,
        package_req: &package::Req,
    ) -> Result<Vec<(package::Name, Vec<package::Version>)>, Error> {
        let mut results = Vec::new();
        for provider in &self.providers {
            results.extend(provider.search(package_req).await?);
        }
        Ok(results)
    }

    /// Find the latest version for a package by name.
    pub(crate) async fn latest_version(
        &self,
        rock_name: &package::Name,
    ) -> Option<package::Version> {
        self.latest_match(&rock_name.clone().into(), None)
            .await
            .map(|result| result.version().clone())
    }

    /// Find the latest package that matches the requirement.
    pub async fn latest_match(
        &self,
        package_req: &package::Req,
        filter: Option<remote::package::kind::FilterSpec>,
    ) -> Option<package::Spec> {
        match self.find(package_req, filter).await {
            Ok(result) => Some(result.spec),
            Err(_) => None,
        }
    }
}
