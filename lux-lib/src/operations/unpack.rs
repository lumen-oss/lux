use async_recursion::async_recursion;
use flate2::read::GzDecoder;
use itertools::Itertools;
use path_slash::PathExt;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::Read;
use std::io::Seek;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs;

use crate::progress::Progress;
use crate::progress::ProgressBar;

#[derive(Error, Debug)]
pub enum UnpackError {
    #[error("failed to unpack source: {0}")]
    Io(#[from] io::Error),
    #[error("failed to unpack zip source: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("source returned HTML - it may have been moved or deleted")]
    SourceMovedOrDeleted,
    #[error("rockspec source has unsupported file type {0}")]
    UnsupportedFileType(String),
    #[error("could not determine mimetype of rockspec source")]
    UnknownMimeType,
}

pub async fn unpack_src_rock<R: Read + Seek + Send>(
    rock_src: R,
    destination: PathBuf,
    progress: &Progress<ProgressBar>,
) -> Result<PathBuf, UnpackError> {
    progress.map(|p| {
        p.set_message(format!(
            "📦 Unpacking src.rock into {}",
            destination.display()
        ))
    });

    unpack_src_rock_impl(rock_src, destination).await
}

async fn unpack_src_rock_impl<R: Read + Seek + Send>(
    rock_src: R,
    destination: PathBuf,
) -> Result<PathBuf, UnpackError> {
    let mut zip = zip::ZipArchive::new(rock_src)?;
    zip.extract(&destination)?;
    Ok(destination)
}

#[async_recursion]
pub(crate) async fn unpack<R>(
    mime_type: Option<&str>,
    reader: R,
    extract_nested_archive: bool,
    file_name: String,
    dest_dir: &Path,
    progress: &Progress<ProgressBar>,
) -> Result<(), UnpackError>
where
    R: Read + Seek + Send,
{
    progress.map(|p| p.set_message(format!("📦 Unpacking {file_name}")));

    match mime_type {
        Some("application/zip") => {
            let mut archive = zip::ZipArchive::new(reader)?;
            archive.extract(dest_dir)?;
        }
        Some("application/x-tar") => {
            let mut archive = tar::Archive::new(reader);
            archive.unpack(dest_dir)?;
        }
        Some("application/gzip") => {
            let mut bufreader = BufReader::new(reader);

            let extract_subdirectory =
                extract_nested_archive && is_single_tar_directory(&mut bufreader)?;

            bufreader.rewind()?;
            let tar = GzDecoder::new(bufreader);
            let mut archive = tar::Archive::new(tar);

            if extract_subdirectory {
                archive.entries()?.try_for_each(|entry| {
                    let mut entry = entry?;

                    let path: PathBuf = entry.path()?.components().skip(1).collect();
                    if path.components().count() > 0 {
                        let dest = dest_dir.join(path);
                        if let Some(dest_parent_dir) = dest.parent() {
                            std::fs::create_dir_all(dest_parent_dir)?;
                        }
                        entry.unpack(dest)?;
                    }

                    Ok::<_, io::Error>(())
                })?;
            } else {
                archive.entries()?.try_for_each(|entry| {
                    entry?.unpack_in(dest_dir)?;
                    Ok::<_, io::Error>(())
                })?;
            }
        }
        Some("text/html") => {
            return Err(UnpackError::SourceMovedOrDeleted);
        }
        Some(other) => {
            return Err(UnpackError::UnsupportedFileType(other.to_string()));
        }
        None => {
            return Err(UnpackError::UnknownMimeType);
        }
    }

    if extract_nested_archive {
        // If the source is an archive, luarocks will pack the source archive and the rockspec.
        // So we need to unpack the source archive.
        if let Some((nested_archive_path, mime_type)) = get_single_archive_entry(dest_dir)? {
            {
                let mut file = File::open(&nested_archive_path)?;
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer)?;
                let file_name = nested_archive_path
                    .file_name()
                    .map(|os_str| os_str.to_string_lossy())
                    .unwrap_or(nested_archive_path.to_string_lossy())
                    .to_string();
                unpack(
                    mime_type,
                    file,
                    extract_nested_archive, // It might be a nested archive inside a .src.rock
                    file_name,
                    dest_dir,
                    progress,
                )
                .await?;
                fs::remove_file(nested_archive_path).await?;
            }
        }
    }
    Ok(())
}

fn is_single_tar_directory<R: Read + Seek + Send>(reader: R) -> io::Result<bool> {
    let tar = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(tar);

    let entries: Vec<_> = archive
        .entries()?
        .filter_map(|entry| {
            if entry.as_ref().ok()?.path().ok()?.file_name()? != "pax_global_header" {
                Some(entry)
            } else {
                None
            }
        })
        .try_collect()?;

    if entries.is_empty() {
        Ok(false)
    } else {
        let directory: PathBuf = entries[0].path()?.components().take(1).collect();

        Ok(entries.into_iter().all(|entry| {
            entry.path().is_ok_and(|path| {
                path.to_slash_lossy()
                    .starts_with(&directory.to_slash_lossy().to_string())
            })
        }))
    }
}

fn get_single_archive_entry(dir: &Path) -> Result<Option<(PathBuf, Option<&str>)>, io::Error> {
    let entries = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|f| {
            let f = f.path();
            if f.extension()
                .is_some_and(|ext| ext.to_string_lossy() != "rockspec")
            {
                Some(f)
            } else {
                None
            }
        })
        .collect_vec();
    if entries.len() != 1 {
        return Ok(None);
    }
    match entries.first() {
        Some(entry) if entry.is_file() => {
            if let mt @ Some(mime_type) =
                infer::get_from_path(entry)?.map(|file_type| file_type.mime_type())
            {
                if matches!(
                    mime_type,
                    "application/zip" | "application/x-tar" | "application/gzip"
                ) {
                    return Ok(Some((entry.clone(), mt)));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use crate::{config::ConfigBuilder, progress::MultiProgress};
    use assert_fs::TempDir;
    use std::fs::File;

    use super::*;

    #[tokio::test]
    pub async fn test_unpack_src_rock() {
        let test_rock_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("test")
            .join("luatest-0.2-1.src.rock");
        let file = File::open(&test_rock_path).unwrap();
        let dest = TempDir::new().unwrap();
        let config = ConfigBuilder::new().unwrap().build().unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        unpack_src_rock(file, dest.to_path_buf(), &bar)
            .await
            .unwrap();
    }
}
