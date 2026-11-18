use std::{
    fs::File,
    io,
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use bon::Builder;
use ignore::gitignore::Gitignore;
use miette::Diagnostic;
use path_slash::PathExt;
use tempfile::TempDir;
use thiserror::Error;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::config::Config;
use crate::fs;
use crate::luarocks;
use crate::operations::fetch;
use crate::operations::FetchSrc;
use crate::reqwest::RequestBuilderExt;
use crate::rockspec::source::Spec;
use crate::rockspec::Rockspec;
use crate::source_tree;

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Source<'a, R: Rockspec> {
    #[builder(start_fn)]
    dest_dir: &'a Path,
    #[builder(start_fn)]
    rockspec: &'a R,
    #[builder(start_fn)]
    rockspec_content: &'a str,
    #[builder(start_fn)]
    config: &'a Config,
}

impl<R: Rockspec, State> SourceBuilder<'_, R, State>
where
    State: source_builder::State + source_builder::IsComplete,
{
    pub async fn pack(self) -> Result<PathBuf, Error> {
        do_pack(self._build()).await
    }
}

#[derive(Error, Debug, Diagnostic)]
pub enum Error {
    #[error("failed to write the source rock at '{}'", path.display())]
    #[diagnostic(help("check that the destination directory is writable"))]
    ZipWrite { path: PathBuf, source: io::Error },
    #[error("failed to copy the source into the source rock")]
    #[diagnostic(help("check that the source tree is readable and the destination is writable"))]
    Copy(#[source] io::Error),
    #[error("failed to write the source rock archive")]
    #[diagnostic(help("check that the destination directory is writable"))]
    Zip(#[from] zip::result::ZipError),
    #[error("failed to walk source tree")]
    #[diagnostic(help("check that the source tree is readable and contains no symlink loops"))]
    Walkdir(#[from] walkdir::Error),
    #[error("failed to determine the source rock path")]
    #[diagnostic(help("this is a bug, please report it"))]
    StripPrefix(#[from] std::path::StripPrefixError),
    #[error(transparent)]
    #[diagnostic(forward(0), help("check network access and source credentials"))]
    Fetch(#[from] fetch::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(forward(0), help("check network access and source credentials"))]
    Request(#[from] crate::reqwest::Error),
    #[error("source type '{0}' is not supported for source rock uploads")]
    #[diagnostic(help("source rocks can be packed from file, URL, or git sources"))]
    UnsupportedSource(String),
}
impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(error.into())
    }
}

enum Content {
    Archive {
        path: PathBuf,
        _dir: Option<TempDir>,
    },
    Directory {
        root: PathBuf,
        prefix: Option<String>,
        _dir: Option<TempDir>,
    },
}

async fn do_pack<R: Rockspec>(source: Source<'_, R>) -> Result<PathBuf, Error> {
    let dest_dir = source.dest_dir;
    let rockspec = source.rockspec;
    let rockspec_content = source.rockspec_content;
    let config = source.config;

    let content = content(rockspec, config).await?;
    write_rock(dest_dir, rockspec, rockspec_content, content)
}

async fn content<R: Rockspec>(rockspec: &R, config: &Config) -> Result<Content, Error> {
    let source_spec = rockspec.source().current_platform().source_spec.clone();
    match source_spec {
        Spec::Url(url) => {
            let request = config
                .http_client()?
                .get(url.clone())
                .apply_access_token(config, &url);
            let bytes = request.send().await?.error_for_status()?.bytes().await?;
            let file_name = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|name| !name.is_empty())
                .unwrap_or("source");
            let dir = fs::tempfile::tempdir()?;
            let path = dir.path().join(file_name);
            fs::sync::write(&path, bytes)?;
            Ok(Content::Archive {
                path,
                _dir: Some(dir),
            })
        }
        Spec::Git(git) => {
            let dir = fs::tempfile::tempdir()?;
            FetchSrc::new(dir.path(), rockspec, config).fetch().await?;
            Ok(Content::Directory {
                root: dir.path().to_path_buf(),
                prefix: Some(git.url.repo().to_string()),
                _dir: Some(dir),
            })
        }
        Spec::File(path) if path.is_dir() => Ok(Content::Directory {
            root: path,
            prefix: Some(format!("{}-{}", rockspec.package(), rockspec.version())),
            _dir: None,
        }),
        Spec::File(path) => Ok(Content::Archive { path, _dir: None }),
        Spec::Wally(_) => Err(Error::UnsupportedSource(source_spec.to_string())),
    }
}

fn write_rock<R: Rockspec>(
    dest_dir: &Path,
    rockspec: &R,
    rockspec_content: &str,
    content: Content,
) -> Result<PathBuf, Error> {
    let package = rockspec.package();
    let version = rockspec.version();
    let rock_path = dest_dir.join(luarocks::rock_file_name(package, version, "src.rock"));
    let file = fs::sync::create(&rock_path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    zip.start_file(format!("{package}-{version}.rockspec"), options)?;
    zip.write_all(rockspec_content.as_bytes())
        .map_err(|source| Error::ZipWrite {
            path: rock_path.clone(),
            source,
        })?;

    match content {
        Content::Archive { path, .. } => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "source".to_string());
            zip.start_file(name, options)?;
            let mut archive = fs::sync::open(&path)?;
            io::copy(&mut archive, &mut zip).map_err(Error::Copy)?;
        }
        Content::Directory { root, prefix, .. } => {
            add_directory(&mut zip, &root, prefix.as_deref())?;
        }
    }

    zip.finish()?;
    Ok(rock_path)
}

fn add_directory(
    zip: &mut ZipWriter<File>,
    root: &Path,
    prefix: Option<&str>,
) -> Result<(), Error> {
    let root = fs::sync::canonicalize(root)?;
    for relative in source_files(&root)? {
        let path = root.join(&relative);
        #[cfg(unix)]
        let options = {
            let metadata = fs::sync::metadata(&path)?;
            SimpleFileOptions::default().unix_permissions(metadata.permissions().mode())
        };
        #[cfg(not(unix))]
        let options = SimpleFileOptions::default();
        let name = match prefix {
            Some(prefix) => format!("{prefix}/{}", relative.to_slash_lossy()),
            None => relative.to_slash_lossy().into_owned(),
        };
        zip.start_file(name, options)?;
        let mut file = fs::sync::open(path)?;
        io::copy(&mut file, zip).map_err(Error::Copy)?;
    }
    Ok(())
}

fn source_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let (gitignore, error) = Gitignore::new(root.join(".gitignore"));
    if let Some(error) = error {
        tracing::warn!(
            "failed to parse '{}': {error}",
            root.join(".gitignore").display()
        );
    }
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(true) {
        let entry = entry?;
        let path = entry.path();
        if entry.path_is_symlink() && !fs::sync::canonicalize(path)?.starts_with(root) {
            tracing::warn!(
                "skipping symlink '{}' which points outside the source root",
                path.display()
            );
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = path.strip_prefix(root)?;
        if !relative.as_os_str().is_empty() && !is_excluded(root, relative, &gitignore) {
            files.push(relative.to_path_buf());
        }
    }
    Ok(files)
}

fn is_excluded(root: &Path, relative: &Path, gitignore: &Gitignore) -> bool {
    let mut first = true;
    for component in relative.components() {
        let name = component.as_os_str().to_string_lossy();
        if source_tree::is_vcs_dir(&name) || source_tree::is_lux_generated(&name) {
            return true;
        }
        if first && name == "target" {
            return true;
        }
        first = false;
    }
    gitignore
        .matched_path_or_any_parents(root.join(relative), false)
        .is_ignore()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use assert_fs::prelude::*;
    use zip::ZipArchive;

    use super::*;
    use crate::config;
    use crate::lua;
    use crate::rockspec;

    #[tokio::test]
    async fn packs_rockspec_and_source_tree() {
        let parent = assert_fs::TempDir::new().unwrap();
        parent
            .child("source/src/foo.lua")
            .write_str("return {}")
            .unwrap();
        parent
            .child("source/src/target/kept.lua")
            .write_str("return 1")
            .unwrap();
        parent.child("source/.jj/repo/store").write_str("").unwrap();
        parent
            .child("source/.lux/lib/dep/foo.lua")
            .write_str("")
            .unwrap();
        parent.child("source/target/foo.o").write_str("").unwrap();
        parent
            .child("source/.gitignore")
            .write_str("secret.txt\nvendor/\n")
            .unwrap();
        parent.child("source/secret.txt").write_str("").unwrap();
        parent
            .child("source/vendor/lib/foo.lua")
            .write_str("")
            .unwrap();
        let content = format!(
            r#"
package = "foo"
version = "1.0.0-1"
source = {{ url = "file://{}" }}
build = {{ type = "builtin" }}
"#,
            parent.path().join("source").display()
        );
        let rockspec = rockspec::lua::Remote::new(&content).unwrap();
        let dest = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::default()
            .data_dir(Some(dest.path().to_path_buf()))
            .cache_dir(Some(dest.path().to_path_buf()))
            .lua_version(Some(lua::Version::Lua51))
            .build()
            .unwrap();

        let rock = Source::new(dest.path(), &rockspec, &content, &config)
            .pack()
            .await
            .unwrap();
        let file = fs::sync::open(&rock).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let names: Vec<_> = (0..zip.len())
            .map(|index| zip.by_index(index).unwrap().name().unwrap().to_string())
            .collect();
        assert!(names.iter().any(|name| name == "foo-1.0.0-1.rockspec"));
        assert!(names.iter().any(|name| name == "foo-1.0.0-1/src/foo.lua"));
        assert!(names
            .iter()
            .any(|name| name == "foo-1.0.0-1/src/target/kept.lua"));
        assert!(!names.iter().any(|name| name.contains("/.jj/")));
        assert!(!names.iter().any(|name| name.contains("/.lux/")));
        assert!(!names
            .iter()
            .any(|name| name.starts_with("foo-1.0.0-1/target/")));
        assert!(!names.iter().any(|name| name == "foo-1.0.0-1/secret.txt"));
        assert!(!names
            .iter()
            .any(|name| name.starts_with("foo-1.0.0-1/vendor/")));

        let mut source = zip.by_name("foo-1.0.0-1/src/foo.lua").unwrap();
        let mut bytes = String::new();
        source.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, "return {}");
    }

    #[tokio::test]
    async fn packs_url_source_archive() {
        let server = httptest::Server::run();
        let mut archive = Vec::new();
        {
            let mut zip = ZipWriter::new(std::io::Cursor::new(&mut archive));
            zip.start_file("foo.lua", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"return {}").unwrap();
            zip.finish().unwrap();
        }
        server.expect(
            httptest::Expectation::matching(httptest::matchers::request::path("/foo-1.0.zip"))
                .respond_with(httptest::responders::status_code(200).body(archive)),
        );
        let content = format!(
            r#"
package = "foo"
version = "1.0.0-1"
source = {{ url = "{}" }}
build = {{ type = "builtin" }}
"#,
            server.url_str("/foo-1.0.zip")
        );
        let rockspec = rockspec::lua::Remote::new(&content).unwrap();
        let dest = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::default()
            .data_dir(Some(dest.path().to_path_buf()))
            .cache_dir(Some(dest.path().to_path_buf()))
            .lua_version(Some(lua::Version::Lua51))
            .build()
            .unwrap();

        let rock = Source::new(dest.path(), &rockspec, &content, &config)
            .pack()
            .await
            .unwrap();
        let file = fs::sync::open(&rock).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let names: Vec<_> = (0..zip.len())
            .map(|index| zip.by_index(index).unwrap().name().unwrap().to_string())
            .collect();
        assert!(names.iter().any(|name| name == "foo-1.0.0-1.rockspec"));
        assert!(names.iter().any(|name| name == "foo-1.0.zip"));
    }
}
