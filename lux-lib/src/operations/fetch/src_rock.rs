use std::io;
use std::io::Cursor;

use miette::Diagnostic;
use thiserror::Error;

use super::RemotePackageSourceMetadata;
use super::SrcRock;
use crate::hash::HasIntegrity;
use crate::lockfile::RemotePackageSourceUrl;
use crate::operations::{download, unpack};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
#[error(transparent)]
pub enum Error {
    DownloadSrcRock(#[from] download::SrcRockError),
    Unpack(#[from] unpack::Error),
    Io(#[from] io::Error),
}

#[tracing::instrument(
    name = "Fetching src.rock",
    level = "info",
    skip_all,
    fields(package = fetch.package.to_string(),),
)]
pub(super) async fn do_fetch_src_rock(
    fetch: SrcRock<'_>,
) -> Result<RemotePackageSourceMetadata, Error> {
    let package = fetch.package;
    let dest_dir = fetch.dest_dir;
    let config = fetch.config;
    let src_rock = download::src_rock(package, config.server(), fetch.config).await?;
    let hash = src_rock.bytes.hash().await?;
    let cursor = Cursor::new(src_rock.bytes);
    let mime_type = infer::get(cursor.get_ref()).map(|file_type| file_type.mime_type());
    unpack::extract(mime_type, cursor, true, src_rock.file_name, dest_dir).await?;
    Ok(RemotePackageSourceMetadata {
        hash,
        source_url: RemotePackageSourceUrl::Url { url: src_rock.url },
    })
}
