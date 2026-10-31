use miette::Diagnostic;
use ssri::Integrity;
use std::io;
use thiserror::Error;

pub mod exec;

use crate::{build, config::Config, fs, lua, operations::unpack, tree::InstallTree};

#[cfg(target_family = "unix")]
use crate::tree::{self, Tree};
#[cfg(target_family = "windows")]
use crate::tree::{self, Tree};

#[cfg(target_family = "unix")]
use crate::pipeline::build_local::Build;

#[cfg(target_family = "unix")]
const LUAROCKS_EXE: &str = "luarocks";
#[cfg(target_family = "windows")]
const LUAROCKS_EXE: &str = "luarocks.exe";

pub(crate) const LUAROCKS_VERSION: &str = "3.13.0-1";

#[cfg(target_family = "unix")]
const LUAROCKS_ROCKSPEC: &str = "
rockspec_format = '3.0'
package = 'luarocks'
version = '3.13.0-1'
source = {
    url = 'git+https://github.com/luarocks/luarocks',
    tag = 'v3.13.0',
}
build = {
    type = 'builtin',
}
";

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] lua::version::Error),
    #[error("IO error occurred while trying to install luarocks")]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Build(#[from] Box<build::Error>),
    #[error(transparent)]
    Request(#[from] crate::reqwest::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Unpack(#[from] unpack::Error),
    #[error("luarocks integrity mismatch.\nExpected: {expected}\nBut got: {got}")]
    IntegrityMismatch { expected: Integrity, got: Integrity },
}

impl From<build::Error> for Error {
    fn from(source: build::Error) -> Self {
        Self::Build(Box::new(source))
    }
}

pub struct Installation {
    tree: Tree,
    config: Config,
}

impl Installation {
    pub fn new(config: &Config, tree: Tree) -> Result<Self, Error> {
        let luarocks_installation = Self {
            tree,
            config: config.clone(),
        };
        Ok(luarocks_installation)
    }

    #[cfg(target_family = "unix")]
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn ensure_installed(&self, lua: &lua::Installation) -> Result<(), Error> {
        use crate::{package, rockspec};

        let mut lockfile = self.tree.lockfile()?.write_guard();

        let luarocks_req = unsafe {
            package::Req::new_unchecked("luarocks".into(), Some(LUAROCKS_VERSION.into()))
        };

        if !self.tree.match_rocks(&luarocks_req)?.is_found() {
            let rockspec =
                unsafe { rockspec::lua::Remote::new(LUAROCKS_ROCKSPEC).unwrap_unchecked() };
            let pkg = Build::new()
                .rockspec(&rockspec)
                .lua(lua)
                .tree(&self.tree)
                .entry_type(tree::EntryType::Entrypoint)
                .config(&self.config)
                .behaviour(build::BuildBehaviour::Ignore)
                .constraint(luarocks_req.version_req().clone().into())
                .build()
                .await?;
            lockfile.add_entrypoint(&pkg);
        }
        Ok(())
    }

    #[cfg(target_family = "windows")]
    #[tracing::instrument(level = "trace", skip(self))]
    pub async fn ensure_installed(&self, _lua: &lua::Installation) -> Result<(), Error> {
        use crate::hash::HasIntegrity;
        use std::io::Cursor;
        let file_name = "luarocks-3.13.0-windows-64";
        let url = format!("https://luarocks.github.io/luarocks/releases/{file_name}.zip");
        let response = reqwest::get(url).await?.error_for_status()?.bytes().await?;
        let hash = response.hash().await?;
        let expected_hash: Integrity = unsafe {
            "sha256-CJet5dRZ1VzRliqUgVN0WmdJ/rNFQDxoqqkgc4hVerk="
                .parse()
                .unwrap_unchecked()
        };
        if expected_hash.matches(&hash).is_none() {
            return Err(Error::IntegrityMismatch {
                expected: expected_hash,
                got: hash,
            });
        }
        let cursor = Cursor::new(response);
        let mime_type = infer::get(cursor.get_ref()).map(|file_type| file_type.mime_type());
        let unpack_dir = fs::tempfile::tempdir()?;
        unpack::extract(
            mime_type,
            cursor,
            false,
            format!("{file_name}.zip"),
            unpack_dir.path(),
        )
        .await?;
        let luarocks_exe = unpack_dir.path().join(file_name).join(LUAROCKS_EXE);
        fs::tokio::copy(luarocks_exe, &self.tree.bin().join(LUAROCKS_EXE)).await?;

        Ok(())
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Self::Request(err.into())
    }
}
