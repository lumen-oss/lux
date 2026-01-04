use std::path::Path;

use bon::Builder;
use thiserror::Error;

use crate::{
    operations::RemoteRockDownload,
    package::PackageReq,
    progress::{Progress, ProgressBar},
};

/// Fetch a vendored rock from `<vendor_dir>`
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct FetchVendored<'a> {
    vendor_dir: &'a Path,
    package: &'a PackageReq,
    progress: &'a Progress<ProgressBar>,
}

#[derive(Error, Debug)]
pub(crate) enum FetchVendoredError {}

impl<State> FetchVendoredBuilder<'_, State>
where
    State: fetch_vendored_builder::State + fetch_vendored_builder::IsComplete,
{
    pub async fn fetch_vendored_rock(self) -> Result<RemoteRockDownload, FetchVendoredError> {
        todo!("construct a VendoredRockDB from vendor_dir and use it to provide `RemoteRockDownload`s")
    }
}
